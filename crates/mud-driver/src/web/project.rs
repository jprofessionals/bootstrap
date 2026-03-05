use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use tokio::net::UnixStream;

use super::build_log::{BuildLog, LogLevel};
use super::static_files::{serve_static, CacheMode};
use crate::mop_rpc::MopRpcClient;

// ---------------------------------------------------------------------------
// Shared state for project routes
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ProjectState {
    world_path: PathBuf,
    build_cache_path: PathBuf,
    #[allow(dead_code)]
    build_log: Arc<BuildLog>,
    /// MOP RPC client for sending access-check requests to the adapter.
    /// `None` when no adapter is connected (e.g. in unit tests).
    #[allow(dead_code)]
    mop_rpc: Option<MopRpcClient>,
    /// Unix socket path for proxying @branch requests to the Ruby portal.
    portal_socket: String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Create the router that serves area web content under `/project/`.
///
/// Routes:
/// - `GET /{ns}/{name}/` — serve area index (SPA dist or static web/)
/// - `GET /{ns}/{name}/{*path}` — serve files from area
///
/// Requests for `@branch` names (e.g. `alice@dev`) are proxied to the Ruby
/// portal's BuilderApp at `/project/`, which enforces access control.
///
/// Serving priority (for non-branch names):
/// 1. SPA mode: `build_cache_path/<ns>-<name>/dist/` — if dist/ exists,
///    serve matching files or fall back to `dist/index.html`.
/// 2. Template mode: `world_path/<ns>/<name>/web/templates/` — if templates/
///    exists, render `.html` files with Tera (context: `area_name`, `namespace`).
/// 3. Static mode: `world_path/<ns>/<name>/web/` — serve static files.
/// 4. 404 if nothing matches.
pub fn project_routes(
    world_path: PathBuf,
    build_cache_path: PathBuf,
    build_log: Arc<BuildLog>,
    mop_rpc: Option<MopRpcClient>,
    portal_socket: String,
) -> Router {
    let state = ProjectState {
        world_path,
        build_cache_path,
        build_log,
        mop_rpc,
        portal_socket,
    };

    Router::new()
        .route("/{ns}/{name}/", any(serve_area_root))
        .route("/{ns}/{name}/{*path}", any(serve_area_file))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Build log API routes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LogsQuery {
    limit: Option<usize>,
    level: Option<String>,
}

/// Create the router for build log API endpoints.
///
/// Routes:
/// - `GET /{ns}/{name}/logs` — returns build logs as a JSON array
///
/// Query params:
/// - `limit` (optional, default 50, max 200)
/// - `level` (optional: "all", "error", "warn", "info" — default "all")
pub fn build_log_routes(build_log: Arc<BuildLog>) -> Router {
    Router::new()
        .route("/{ns}/{name}/logs", get(logs_handler))
        .with_state(build_log)
}

async fn logs_handler(
    Path((ns, name)): Path<(String, String)>,
    Query(params): Query<LogsQuery>,
    State(build_log): State<Arc<BuildLog>>,
) -> impl IntoResponse {
    let area_key = format!("{ns}/{name}");
    let limit = params.limit.unwrap_or(50).min(200);
    let level = match params.level.as_deref() {
        Some("error") => Some(LogLevel::Error),
        Some("warn") => Some(LogLevel::Warn),
        Some("info") => Some(LogLevel::Info),
        _ => None,
    };
    let entries = build_log.recent(&area_key, limit, level);
    Json(entries)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn serve_area_root(
    State(state): State<ProjectState>,
    Path((ns, name)): Path<(String, String)>,
    req: Request,
) -> Response {
    // Proxy @branch requests to Ruby portal for access control + API routing.
    // If Ruby returns 302/403 (auth failure), return that response.
    // If Ruby returns 404 (auth OK but no content), serve from Rust.
    if name.contains('@') {
        let headers = req.headers().clone();
        let resp = proxy_request_to_portal(&state.portal_socket, req).await;
        let status = resp.status().as_u16();
        if status != 404 {
            log_proxy_error(&state, &ns, &name, status, "/");
            return resp;
        }
        // Auth passed — serve content from Rust
        return serve_area_path(&state, &ns, &name, "", &headers).await;
    }
    serve_area_path(&state, &ns, &name, "", req.headers()).await
}

async fn serve_area_file(
    State(state): State<ProjectState>,
    Path((ns, name, path)): Path<(String, String, String)>,
    req: Request,
) -> Response {
    // Proxy @branch requests to Ruby portal for access control + API routing.
    if name.contains('@') {
        let headers = req.headers().clone();
        let resp = proxy_request_to_portal(&state.portal_socket, req).await;
        let status = resp.status().as_u16();
        if status != 404 {
            log_proxy_error(&state, &ns, &name, status, &path);
            return resp;
        }
        // API routes are fully owned by Ruby — never fall through to static files.
        if path.starts_with("api/") || path == "api" {
            return StatusCode::NOT_FOUND.into_response();
        }
        return serve_area_path(&state, &ns, &name, &path, &headers).await;
    }
    serve_area_path(&state, &ns, &name, &path, req.headers()).await
}

async fn serve_area_path(
    state: &ProjectState,
    ns: &str,
    name: &str,
    path: &str,
    headers: &HeaderMap,
) -> Response {
    // Reject paths with traversal components early
    if path.split('/').any(|seg| seg == ".." || seg == ".") {
        return StatusCode::NOT_FOUND.into_response();
    }

    let area_key = format!("{}-{}", ns, name);

    // 1. Try SPA mode: build_cache/<ns>-<name>/dist/
    let dist_dir = state.build_cache_path.join(&area_key).join("dist");
    if dist_dir.is_dir() {
        let file_path = if path.is_empty() {
            dist_dir.join("index.html")
        } else {
            dist_dir.join(path)
        };

        // Path traversal protection: canonicalize and check prefix
        if let Ok(canonical) = file_path.canonicalize() {
            let dist_canonical = dist_dir.canonicalize().unwrap_or_default();
            if canonical.starts_with(&dist_canonical) {
                if canonical.is_file() {
                    let cache_mode = if path.contains("assets/") {
                        CacheMode::Fingerprinted
                    } else {
                        CacheMode::NoCache
                    };
                    match serve_static(&canonical, cache_mode, headers).await {
                        Ok(resp) => return resp,
                        Err(status) => return status.into_response(),
                    }
                }
            } else {
                // Traversal attempt — return 404
                return StatusCode::NOT_FOUND.into_response();
            }
        }

        // SPA fallback: if file doesn't exist, serve index.html.
        // Skip fallback for /api/ paths — those should 404 if unhandled.
        if !path.starts_with("api/") {
            let index_path = dist_dir.join("index.html");
            if let Ok(canonical) = index_path.canonicalize() {
                if canonical.is_file() {
                    match serve_static(&canonical, CacheMode::NoCache, headers).await {
                        Ok(resp) => return resp,
                        Err(status) => return status.into_response(),
                    }
                }
            }
        }
    }

    // 2. Try template mode: world/<ns>/<name>/web/templates/
    let templates_dir = state.world_path.join(ns).join(name).join("web/templates");
    if templates_dir.is_dir() {
        // Only render index.html for the root request
        let template_file = if path.is_empty() {
            "index.html"
        } else {
            path
        };

        let template_path = templates_dir.join(template_file);
        if template_path.is_file() && template_file.ends_with(".html") {
            match tera::Tera::new(&format!("{}/**/*.html", templates_dir.display())) {
                Ok(tera) => {
                    let mut ctx = tera::Context::new();
                    ctx.insert("area_name", name);
                    ctx.insert("namespace", ns);
                    // Provide defaults for template variables that the
                    // adapter would normally supply via GetWebData MOP RPC
                    // (not yet wired up — TODO).
                    ctx.insert("room_count", &0);
                    ctx.insert("item_count", &0);
                    ctx.insert("npc_count", &0);
                    ctx.insert("server_name", "MUD Driver");
                    ctx.insert("players_online", &0);
                    match tera.render(template_file, &ctx) {
                        Ok(html) => {
                            return (
                                StatusCode::OK,
                                [("content-type", "text/html; charset=utf-8")],
                                html,
                            )
                                .into_response();
                        }
                        Err(e) => {
                            tracing::error!("Tera render error for {ns}/{name}: {e}");
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Template render error: {e}"),
                            )
                                .into_response();
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Tera init error for {ns}/{name}: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Template init error: {e}"),
                    )
                        .into_response();
                }
            }
        }
    }

    // 3. Try static mode: world/<ns>/<name>/web/
    let web_dir = state.world_path.join(ns).join(name).join("web");
    if web_dir.is_dir() {
        let file_path = if path.is_empty() {
            web_dir.join("index.html")
        } else {
            web_dir.join(path)
        };

        if let Ok(canonical) = file_path.canonicalize() {
            let web_canonical = web_dir.canonicalize().unwrap_or_default();
            if canonical.starts_with(&web_canonical) && canonical.is_file() {
                match serve_static(&canonical, CacheMode::Development, headers).await {
                    Ok(resp) => return resp,
                    Err(status) => return status.into_response(),
                }
            }
        }

        return StatusCode::NOT_FOUND.into_response();
    }

    // 4. Nothing matched
    StatusCode::NOT_FOUND.into_response()
}

// ---------------------------------------------------------------------------
// Proxy error logging
// ---------------------------------------------------------------------------

/// Log 5xx responses from the portal proxy to the area's build log so they
/// appear in the editor's log panel.
fn log_proxy_error(state: &ProjectState, ns: &str, name: &str, status: u16, path: &str) {
    if status >= 500 {
        // Strip the @branch suffix to get the area key (e.g. "test/test@dev" -> "test/test")
        let area_name = name.split('@').next().unwrap_or(name);
        let area_key = format!("{ns}/{area_name}");
        state.build_log.append(
            &area_key,
            LogLevel::Error,
            "web_app",
            &format!("{status} on /{path}"),
        );
    }
}

// ---------------------------------------------------------------------------
// Portal proxy for @branch requests
// ---------------------------------------------------------------------------

/// Proxy a request to the Ruby portal at `/project/...`.
///
/// Note: when this handler runs inside `nest_service("/project", ...)`,
/// the `/project` prefix has already been stripped by axum.  The remaining
/// path is e.g. `/alice/alice@dev/`, so we prepend `/project` to restore it.
async fn proxy_request_to_portal(socket_path: &str, req: Request) -> Response {
    let uri_path = req.uri().path();
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let path_and_query = format!("/project{uri_path}{query}");
    let method = req.method().clone();
    let headers = req.headers().clone();
    let body = req.into_body();
    proxy_to_portal(socket_path, method, &path_and_query, &headers, body).await
}

/// Proxy a request to the Ruby portal via Unix socket.
///
/// Forwards the request at `/project/...` to the Ruby portal, which
/// mounts BuilderApp at both `/builder/` and `/project/`.
async fn proxy_to_portal(
    socket_path: &str,
    method: axum::http::Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: Body,
) -> Response {
    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, path = %socket_path, "portal socket connect failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Portal unavailable: {e}")))
                .unwrap();
        }
    };

    let io = TokioIo::new(stream);

    let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
        Ok(parts) => parts,
        Err(e) => {
            tracing::error!(error = %e, "portal handshake failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Handshake failed: {e}")))
                .unwrap();
        }
    };

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!(error = %e, "portal proxy connection error");
        }
    });

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path_and_query);

    for (name, value) in headers.iter() {
        if name == axum::http::header::HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder = builder.header("host", "localhost");

    let upstream_req = builder.body(body).unwrap();

    let upstream_resp = match sender.send_request(upstream_req).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error = %e, "portal proxy request failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Proxy request failed: {e}")))
                .unwrap();
        }
    };

    let (parts, body) = upstream_resp.into_parts();
    let body = Body::new(body.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));

    let mut response = Response::builder().status(parts.status);
    for (name, value) in parts.headers.iter() {
        response = response.header(name, value);
    }

    response.body(body).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_build_logs_api() {
        let build_log = Arc::new(BuildLog::new(100));
        build_log.append("ns/myarea", LogLevel::Info, "spa_build", "npm install succeeded");
        build_log.append("ns/myarea", LogLevel::Error, "spa_build", "vite build failed");

        let app = build_log_routes(build_log);

        // All logs
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/ns/myarea/logs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 2);

        // Filter by error
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/ns/myarea/logs?level=error")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["level"], "error");

        // Filter by info
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/ns/myarea/logs?level=info")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["level"], "info");

        // Empty area
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/ns/other/logs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_build_logs_api_limit() {
        let build_log = Arc::new(BuildLog::new(500));
        for i in 0..100 {
            build_log.append("ns/area", LogLevel::Info, "build", &format!("step {i}"));
        }

        let app = build_log_routes(build_log);

        // Default limit is 50
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/ns/area/logs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 50);

        // Custom limit
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/ns/area/logs?limit=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 10);

        // Limit capped at 200
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/ns/area/logs?limit=999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 100); // only 100 entries exist
    }
}
