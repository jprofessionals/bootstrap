use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};

use crate::web::ai_providers::{self, AiProvider, StreamRequest};
use crate::web::server::AppState;
use crate::web::session::BuilderUser;

// ---------------------------------------------------------------------------
// Route construction
// ---------------------------------------------------------------------------

/// Build the router for AI proxy endpoints.
///
/// Meant to be nested under `/api/ai` in the main application.
pub fn ai_routes() -> Router<AppState> {
    Router::new()
        .route("/stream", post(stream_handler))
        .route("/apikey", post(store_apikey_handler))
        .route("/apikey", get(check_apikey_handler))
        .route("/apikey", delete(delete_apikey_handler))
        .route("/preferences", get(get_preferences_handler))
        .route("/preferences", post(set_preferences_handler))
        .route("/models", get(list_models_handler))
        .route("/skills", get(list_skills_handler))
        .route("/skills/{name}", get(get_skill_handler))
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct StoreApiKeyRequest {
    api_key: String,
    #[serde(default = "default_provider")]
    provider: String,
}

fn default_provider() -> String {
    "anthropic".into()
}

#[derive(Debug, Deserialize)]
struct DeleteApiKeyRequest {
    #[serde(default = "default_provider")]
    provider: String,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    status: String,
}

#[derive(Debug, Serialize)]
struct ApiKeyStatusResponse {
    providers: HashMap<String, bool>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct PreferencesRequest {
    default_provider: String,
    #[serde(default)]
    default_model: Option<String>,
}

#[derive(Debug, Serialize)]
struct PreferencesResponse {
    default_provider: String,
    default_model: Option<String>,
}

#[derive(Debug, Serialize)]
struct ModelsResponse {
    providers: HashMap<String, Vec<ai_providers::ModelInfo>>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(ErrorResponse { error: message.into() })).into_response()
}

// ---------------------------------------------------------------------------
// Stream handler — provider-aware SSE proxy
// ---------------------------------------------------------------------------

async fn stream_handler(
    user: BuilderUser,
    State(state): State<AppState>,
    Json(req): Json<StreamRequest>,
) -> Response {
    let ai_key_store = match &state.ai_key_store {
        Some(store) => Arc::clone(store),
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "AI key store not configured",
            );
        }
    };

    // Determine which provider to use: request > preferences > "anthropic"
    let provider_name = if let Some(ref p) = req.provider {
        p.clone()
    } else {
        match ai_key_store.get_preferences(&user.player_id).await {
            Ok((prov, _)) => prov,
            Err(_) => "anthropic".to_string(),
        }
    };

    // Get the provider adapter
    let provider = match ai_providers::get_provider(&provider_name) {
        Some(p) => p,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unknown provider: {}", provider_name),
            );
        }
    };

    // Look up the builder's API key for this provider
    let api_key = match ai_key_store
        .get_ai_api_key(&user.player_id, &provider_name)
        .await
    {
        Ok(Some(key)) => key,
        Ok(None) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("No API key configured for provider: {}", provider_name),
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to retrieve AI API key");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to retrieve API key",
            );
        }
    };

    // Build the provider-specific request
    let mut provider_req = provider.build_request(&api_key, &req);

    // Apply base URL override from config (Anthropic only for now)
    if provider_name == "anthropic" {
        if let Some(ref base) = state.anthropic_base_url {
            let base = base.trim_end_matches('/');
            provider_req.url = format!("{}/v1/messages", base);
        }
    }

    // Build the reqwest request from ProviderRequest
    let mut request_builder = state
        .http_client
        .post(&provider_req.url);

    for (key, value) in &provider_req.headers {
        request_builder = request_builder.header(key.as_str(), value.as_str());
    }

    let upstream_resp = match request_builder.json(&provider_req.body).send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error = %e, provider = %provider_name, "failed to connect to AI API");
            return error_response(StatusCode::BAD_GATEWAY, "Failed to connect to AI API");
        }
    };

    // If the upstream returned an error status, forward it
    if !upstream_resp.status().is_success() {
        let status = upstream_resp.status().as_u16();
        let error_body = upstream_resp
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".into());
        tracing::warn!(status, body = %error_body, provider = %provider_name, "AI API error");
        return error_response(
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            error_body,
        );
    }

    // Stream SSE events from the provider back to the browser
    let byte_stream = upstream_resp.bytes_stream();
    let event_stream = sse_forward_stream(byte_stream, provider);

    Sse::new(event_stream).into_response()
}

/// Convert a byte stream from an upstream SSE response into axum SSE events.
///
/// Each raw SSE event is passed through `provider.translate_event()` which
/// converts provider-specific formats into Anthropic-compatible events that
/// the browser client expects.
fn sse_forward_stream(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
    provider: Box<dyn AiProvider>,
) -> impl Stream<Item = Result<Event, Infallible>> + Send + 'static {
    use futures::StreamExt;

    // We buffer parsed events from translate_event() since it can return
    // multiple events per raw SSE event.
    futures::stream::unfold(
        (
            Box::pin(byte_stream),
            String::new(),
            Vec::<u8>::new(),
            String::new(),
            String::new(),
            Vec::<ai_providers::AnthropicSseEvent>::new(),
            provider,
        ),
        |(mut stream, mut buf, mut raw_buf, mut current_event, mut current_data, mut pending, mut provider)| async move {
            loop {
                // Drain any pending translated events first
                if let Some(translated) = pending.pop() {
                    let sse_event = Event::default()
                        .event(translated.event_type)
                        .data(translated.data);
                    return Some((
                        Ok(sse_event),
                        (stream, buf, raw_buf, current_event, current_data, pending, provider),
                    ));
                }

                // Try to extract a complete SSE event from the buffer
                while let Some(line_end) = buf.find('\n') {
                    let line = buf[..line_end].trim_end_matches('\r').to_string();
                    buf = buf[line_end + 1..].to_string();

                    if line.is_empty() {
                        // Empty line = end of SSE event
                        if !current_data.is_empty() || !current_event.is_empty() {
                            let event_type = if current_event.is_empty() {
                                "message".to_string()
                            } else {
                                std::mem::take(&mut current_event)
                            };
                            let data = std::mem::take(&mut current_data);

                            // Translate through the provider adapter
                            let mut translated = provider.translate_event(&event_type, &data);
                            // Reverse so we can pop from the end in order
                            translated.reverse();
                            pending = translated;

                            // Drain first event immediately
                            if let Some(evt) = pending.pop() {
                                let sse_event = Event::default()
                                    .event(evt.event_type)
                                    .data(evt.data);
                                return Some((
                                    Ok(sse_event),
                                    (stream, buf, raw_buf, current_event, current_data, pending, provider),
                                ));
                            }
                        }
                    } else if let Some(rest) = line.strip_prefix("event:") {
                        current_event = rest.trim_start().to_string();
                    } else if let Some(rest) = line.strip_prefix("data:") {
                        let data_part = rest.trim_start();
                        if !current_data.is_empty() {
                            current_data.push('\n');
                        }
                        current_data.push_str(data_part);
                    }
                    // Ignore other lines (comments starting with ':', etc.)
                }

                // Need more data from the upstream stream
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        // Buffer raw bytes to handle multi-byte UTF-8 split across chunks
                        raw_buf.extend_from_slice(&chunk);
                        // Find the longest valid UTF-8 prefix
                        match std::str::from_utf8(&raw_buf) {
                            Ok(s) => {
                                buf.push_str(s);
                                raw_buf.clear();
                            }
                            Err(e) => {
                                let valid_up_to = e.valid_up_to();
                                if valid_up_to > 0 {
                                    // Safety: we just verified this prefix is valid UTF-8
                                    let valid = unsafe { std::str::from_utf8_unchecked(&raw_buf[..valid_up_to]) };
                                    buf.push_str(valid);
                                }
                                // Keep the trailing incomplete bytes for the next chunk
                                raw_buf = raw_buf[valid_up_to..].to_vec();
                            }
                        }
                    }
                    Some(Err(e)) => {
                        tracing::error!(error = %e, "error reading AI SSE stream");
                        let event = Event::default()
                            .event("error")
                            .data(format!("Upstream error: {}", e));
                        return Some((
                            Ok(event),
                            (stream, String::new(), Vec::new(), String::new(), String::new(), Vec::new(), provider),
                        ));
                    }
                    None => {
                        // Stream ended. Flush any remaining data as a final event.
                        if !current_data.is_empty() || !current_event.is_empty() {
                            let event_type = if current_event.is_empty() {
                                "message".to_string()
                            } else {
                                current_event
                            };
                            let data = current_data;

                            let mut translated = provider.translate_event(&event_type, &data);
                            translated.reverse();
                            if let Some(evt) = translated.pop() {
                                // Put remaining back as pending (though stream is ending)
                                pending = translated;
                                let sse_event = Event::default()
                                    .event(evt.event_type)
                                    .data(evt.data);
                                return Some((
                                    Ok(sse_event),
                                    (stream, String::new(), Vec::new(), String::new(), String::new(), pending, provider),
                                ));
                            }
                        }
                        return None;
                    }
                }
            }
        },
    )
}

// ---------------------------------------------------------------------------
// API key management handlers
// ---------------------------------------------------------------------------

async fn store_apikey_handler(
    user: BuilderUser,
    State(state): State<AppState>,
    Json(req): Json<StoreApiKeyRequest>,
) -> Response {
    tracing::info!(player = %user.player_id, provider = %req.provider, "store_apikey_handler called");
    let ai_key_store = match &state.ai_key_store {
        Some(store) => store,
        None => {
            tracing::error!("AI key store is None — check database.encryption_key in config");
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "AI key store not configured (missing encryption_key in config)",
            );
        }
    };

    match ai_key_store
        .store_ai_api_key(&user.player_id, &req.provider, &req.api_key)
        .await
    {
        Ok(()) => Json(StatusResponse {
            status: "ok".into(),
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to store AI API key");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store API key",
            )
        }
    }
}

async fn check_apikey_handler(
    user: BuilderUser,
    State(state): State<AppState>,
) -> Response {
    let ai_key_store = match &state.ai_key_store {
        Some(store) => store,
        None => {
            return Json(ApiKeyStatusResponse {
                providers: HashMap::new(),
            })
            .into_response();
        }
    };

    let stored_providers = match ai_key_store.list_providers(&user.player_id).await {
        Ok(providers) => providers,
        Err(e) => {
            tracing::error!(error = %e, "failed to list AI API key providers");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to check API keys",
            );
        }
    };

    let mut providers = HashMap::new();
    for name in &["anthropic", "openai", "gemini"] {
        providers.insert(
            name.to_string(),
            stored_providers.contains(&name.to_string()),
        );
    }

    Json(ApiKeyStatusResponse { providers }).into_response()
}

async fn delete_apikey_handler(
    user: BuilderUser,
    State(state): State<AppState>,
    Json(req): Json<DeleteApiKeyRequest>,
) -> Response {
    let ai_key_store = match &state.ai_key_store {
        Some(store) => store,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "AI key store not configured",
            );
        }
    };

    match ai_key_store
        .delete_ai_api_key(&user.player_id, &req.provider)
        .await
    {
        Ok(()) => Json(StatusResponse {
            status: "ok".into(),
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete AI API key");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to delete API key",
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Preferences handlers
// ---------------------------------------------------------------------------

async fn get_preferences_handler(
    user: BuilderUser,
    State(state): State<AppState>,
) -> Response {
    let ai_key_store = match &state.ai_key_store {
        Some(store) => store,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "AI key store not configured",
            );
        }
    };

    match ai_key_store.get_preferences(&user.player_id).await {
        Ok((default_provider, default_model)) => Json(PreferencesResponse {
            default_provider,
            default_model,
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to get AI preferences");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get preferences",
            )
        }
    }
}

async fn set_preferences_handler(
    user: BuilderUser,
    State(state): State<AppState>,
    Json(req): Json<PreferencesRequest>,
) -> Response {
    let ai_key_store = match &state.ai_key_store {
        Some(store) => store,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "AI key store not configured",
            );
        }
    };

    // Validate that the provider is known
    if ai_providers::get_provider(&req.default_provider).is_none() {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Unknown provider: {}", req.default_provider),
        );
    }

    match ai_key_store
        .set_preferences(
            &user.player_id,
            &req.default_provider,
            req.default_model.as_deref(),
        )
        .await
    {
        Ok(()) => Json(StatusResponse {
            status: "ok".into(),
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to set AI preferences");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to set preferences",
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Models handler
// ---------------------------------------------------------------------------

async fn list_models_handler() -> Response {
    let mut providers = HashMap::new();

    for name in &["anthropic", "openai", "gemini"] {
        if let Some(provider) = ai_providers::get_provider(name) {
            providers.insert(name.to_string(), provider.models());
        }
    }

    Json(ModelsResponse { providers }).into_response()
}

// ---------------------------------------------------------------------------
// Skills handlers
// ---------------------------------------------------------------------------

async fn list_skills_handler(State(state): State<AppState>) -> Response {
    let skills_service = match &state.skills_service {
        Some(svc) => svc,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Skills service not configured",
            );
        }
    };

    let skills = skills_service.list_skills().await;
    Json(skills).into_response()
}

async fn get_skill_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let skills_service = match &state.skills_service {
        Some(svc) => svc,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Skills service not configured",
            );
        }
    };

    match skills_service.get_skill(&name).await {
        Some(skill) => Json(skill).into_response(),
        None => error_response(
            StatusCode::NOT_FOUND,
            format!("Skill '{}' not found", name),
        ),
    }
}
