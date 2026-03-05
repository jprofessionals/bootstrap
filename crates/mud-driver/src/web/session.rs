use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use hyper_util::rt::TokioIo;
use http_body_util::BodyExt;
use serde::Deserialize;
use tokio::net::UnixStream;

use crate::web::server::AppState;

// ---------------------------------------------------------------------------
// Session validation via Ruby portal
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WhoamiResponse {
    player_id: String,
    role: String,
}

/// Validate the user's session by calling the Ruby portal's `/account/api/whoami`
/// endpoint via the Unix domain socket, forwarding the browser's cookies.
async fn validate_portal_session(
    portal_socket: &str,
    cookie_header: Option<&str>,
) -> Option<WhoamiResponse> {
    let stream = match UnixStream::connect(portal_socket).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, socket = %portal_socket, "portal session check: socket connect failed");
            return None;
        }
    };
    let io = TokioIo::new(stream);

    let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
        Ok(parts) => parts,
        Err(e) => {
            tracing::warn!(error = %e, "portal session check: handshake failed");
            return None;
        }
    };

    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri("/account/api/whoami")
        .header("host", "localhost");

    if let Some(cookies) = cookie_header {
        builder = builder.header("cookie", cookies);
    }

    let req = builder
        .body(axum::body::Body::empty())
        .ok()?;

    let resp = match sender.send_request(req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "portal session check: request failed");
            return None;
        }
    };

    if resp.status() != 200 {
        tracing::debug!(status = %resp.status(), "portal session check: not authenticated");
        return None;
    }

    let body = resp.into_body().collect().await.ok()?.to_bytes();
    match serde_json::from_slice(&body) {
        Ok(whoami) => Some(whoami),
        Err(e) => {
            tracing::warn!(error = %e, body = %String::from_utf8_lossy(&body), "portal session check: invalid JSON response");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// AuthUser — requires a logged-in session (validated via Ruby portal)
// ---------------------------------------------------------------------------

/// Represents an authenticated user session.
///
/// Validates by calling the Ruby portal's `/account/api/whoami` endpoint
/// via the Unix domain socket, forwarding the browser's cookies.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub player_id: String,
    pub role: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookie_header = parts
            .headers
            .get("cookie")
            .and_then(|v| v.to_str().ok());

        match validate_portal_session(&state.portal_socket, cookie_header).await {
            Some(whoami) => Ok(AuthUser {
                player_id: whoami.player_id,
                role: whoami.role,
            }),
            None => Err(StatusCode::UNAUTHORIZED.into_response()),
        }
    }
}

// ---------------------------------------------------------------------------
// BuilderUser — requires builder (non-player) role
// ---------------------------------------------------------------------------

/// Represents a builder session (any role other than `"player"`).
///
/// Validates via the Ruby portal and additionally checks that the user
/// has a builder role.
#[derive(Debug, Clone)]
pub struct BuilderUser {
    pub player_id: String,
    pub role: String,
}

impl FromRequestParts<AppState> for BuilderUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth = AuthUser::from_request_parts(parts, state).await?;
        if auth.role == "player" {
            return Err(StatusCode::FORBIDDEN.into_response());
        }
        Ok(BuilderUser {
            player_id: auth.player_id,
            role: auth.role,
        })
    }
}
