use std::path::{Path, PathBuf};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::mop_rpc::MopRpcClient;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum file size for writes (1 MiB).
const MAX_FILE_SIZE: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct EditorState {
    world_path: PathBuf,
    /// MOP RPC client for sending access-check requests to the adapter.
    /// `None` when no adapter is connected (e.g. in unit tests).
    #[allow(dead_code)]
    mop_rpc: Option<MopRpcClient>,
    portal_socket: String,
}

// ---------------------------------------------------------------------------
// Query / body types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RepoQuery {
    repo: String,
}

#[derive(Debug, Deserialize)]
struct FileBody {
    content: String,
}

#[derive(Debug, Serialize)]
struct ListResponse {
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReadResponse {
    content: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

// ---------------------------------------------------------------------------
// Router constructor
// ---------------------------------------------------------------------------

/// Build the router for editor file CRUD endpoints.
///
/// Intended to be nested at `/api/editor` in the main application.
pub fn editor_file_routes(world_path: PathBuf, mop_rpc: Option<MopRpcClient>, portal_socket: String) -> Router {
    let state = EditorState { world_path, mop_rpc, portal_socket };

    Router::new()
        .route("/files", get(list_files_handler))
        .route(
            "/files/{*path}",
            get(read_file_handler)
                .put(write_file_handler)
                .post(create_file_handler)
                .delete(delete_file_handler),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Path resolution helpers
// ---------------------------------------------------------------------------

/// Parse `"ns/name"` from the `repo` query parameter and return the workspace
/// directory, preferring `<name>@dev` if it exists, falling back to `<name>`.
fn resolve_workspace(world_path: &Path, repo: &str) -> Result<PathBuf, Response> {
    let parts: Vec<&str> = repo.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "repo must be in the form namespace/name",
        ));
    }
    let (ns, name) = (parts[0], parts[1]);

    let dev_path = world_path.join(ns).join(format!("{}@dev", name));
    if dev_path.is_dir() {
        return Ok(dev_path);
    }

    let prod_path = world_path.join(ns).join(name);
    if prod_path.is_dir() {
        return Ok(prod_path);
    }

    Err(error_response(
        StatusCode::NOT_FOUND,
        format!("workspace not found: {}", repo),
    ))
}

/// Resolve a relative file path within a workspace, blocking path traversal
/// and `.git/` access.
fn resolve_file_path(workspace: &Path, rel_path: &str) -> Result<PathBuf, Response> {
    // Block .git access
    if rel_path == ".git"
        || rel_path.starts_with(".git/")
        || rel_path.contains("/.git/")
        || rel_path.contains("/.git")
    {
        return Err(error_response(StatusCode::FORBIDDEN, "access to .git is forbidden"));
    }

    // Build the candidate path and canonicalize to catch traversal.
    // The workspace itself must exist (it does — we just resolved it).
    let candidate = workspace.join(rel_path);

    // Use the canonical workspace path for the starts_with check.
    let canonical_workspace = workspace.canonicalize().map_err(|_| {
        error_response(StatusCode::INTERNAL_SERVER_ERROR, "workspace canonicalize failed")
    })?;

    // For existing paths we canonicalize; for new paths we canonicalize the
    // parent and append the file name.
    let canonical = if candidate.exists() {
        candidate.canonicalize().map_err(|_| {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "path canonicalize failed")
        })?
    } else {
        // Parent must exist (or we create it).
        let parent = candidate.parent().ok_or_else(|| {
            error_response(StatusCode::BAD_REQUEST, "invalid path")
        })?;
        if parent.exists() {
            let canonical_parent = parent.canonicalize().map_err(|_| {
                error_response(StatusCode::BAD_REQUEST, "invalid parent path")
            })?;
            let file_name = candidate.file_name().ok_or_else(|| {
                error_response(StatusCode::BAD_REQUEST, "invalid file name")
            })?;
            canonical_parent.join(file_name)
        } else {
            // For deeply nested new files, walk up to find an existing ancestor.
            let mut existing = parent.to_path_buf();
            let mut tail_parts = Vec::new();
            while !existing.exists() {
                if let Some(name) = existing.file_name() {
                    tail_parts.push(name.to_os_string());
                } else {
                    return Err(error_response(StatusCode::BAD_REQUEST, "invalid path"));
                }
                existing = existing
                    .parent()
                    .ok_or_else(|| error_response(StatusCode::BAD_REQUEST, "invalid path"))?
                    .to_path_buf();
            }
            let mut resolved = existing.canonicalize().map_err(|_| {
                error_response(StatusCode::BAD_REQUEST, "invalid path")
            })?;
            for part in tail_parts.into_iter().rev() {
                resolved.push(part);
            }
            if let Some(file_name) = candidate.file_name() {
                resolved.push(file_name);
            }
            resolved
        }
    };

    if !canonical.starts_with(&canonical_workspace) {
        return Err(error_response(StatusCode::FORBIDDEN, "path traversal denied"));
    }

    Ok(canonical)
}

// ---------------------------------------------------------------------------
// Session validation
// ---------------------------------------------------------------------------

/// Validate the session cookie by calling the Ruby portal's `/account/api/whoami`
/// endpoint via the Unix domain socket. Returns `Ok(())` if the user has builder
/// (or admin) access, or an error response otherwise.
async fn validate_builder_session(portal_socket: &str, cookie_header: Option<&str>) -> Result<(), Response> {
    // Skip auth when no portal socket is configured (e.g. in unit tests).
    if portal_socket.is_empty() {
        return Ok(());
    }

    let stream = match tokio::net::UnixStream::connect(portal_socket).await {
        Ok(s) => s,
        Err(_) => return Err(error_response(StatusCode::UNAUTHORIZED, "authentication required")),
    };
    let io = hyper_util::rt::TokioIo::new(stream);

    let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
        Ok(parts) => parts,
        Err(_) => return Err(error_response(StatusCode::UNAUTHORIZED, "authentication required")),
    };

    tokio::spawn(async move { let _ = conn.await; });

    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri("/account/api/whoami")
        .header("host", "localhost");

    if let Some(cookies) = cookie_header {
        builder = builder.header("cookie", cookies);
    }

    let req = builder.body(axum::body::Body::empty())
        .map_err(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "request build failed"))?;

    let resp = sender.send_request(req).await
        .map_err(|_| error_response(StatusCode::UNAUTHORIZED, "authentication required"))?;

    if resp.status() != hyper::StatusCode::OK {
        return Err(error_response(StatusCode::UNAUTHORIZED, "authentication required"));
    }

    // Check if user has builder role
    use http_body_util::BodyExt;
    let body = resp.into_body().collect().await
        .map_err(|_| error_response(StatusCode::UNAUTHORIZED, "authentication required"))?
        .to_bytes();

    #[derive(serde::Deserialize)]
    struct WhoamiResp { role: String }

    let whoami: WhoamiResp = serde_json::from_slice(&body)
        .map_err(|_| error_response(StatusCode::UNAUTHORIZED, "authentication required"))?;

    if whoami.role == "player" {
        return Err(error_response(StatusCode::FORBIDDEN, "builder access required"));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_files_handler(
    State(state): State<EditorState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<RepoQuery>,
) -> Response {
    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok());
    if let Err(e) = validate_builder_session(&state.portal_socket, cookie).await {
        return e;
    }

    let workspace = match resolve_workspace(&state.world_path, &query.repo) {
        Ok(ws) => ws,
        Err(e) => return e,
    };

    let mut files = Vec::new();
    if let Err(e) = collect_files(&workspace, &workspace, &mut files) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to list files: {}", e),
        );
    }

    files.sort();
    Json(ListResponse { files }).into_response()
}

/// Recursively collect files, excluding `.git/`.
fn collect_files(
    base: &Path,
    dir: &Path,
    out: &mut Vec<String>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == ".git" {
            continue;
        }

        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out)?;
        } else {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    Ok(())
}

async fn read_file_handler(
    State(state): State<EditorState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<RepoQuery>,
    axum::extract::Path(rel_path): axum::extract::Path<String>,
) -> Response {
    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok());
    if let Err(e) = validate_builder_session(&state.portal_socket, cookie).await {
        return e;
    }

    let workspace = match resolve_workspace(&state.world_path, &query.repo) {
        Ok(ws) => ws,
        Err(e) => return e,
    };

    let file_path = match resolve_file_path(&workspace, &rel_path) {
        Ok(p) => p,
        Err(e) => return e,
    };

    match std::fs::read_to_string(&file_path) {
        Ok(content) => Json(ReadResponse {
            content,
            path: rel_path,
        })
        .into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            error_response(StatusCode::NOT_FOUND, "file not found")
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read failed: {}", e),
        ),
    }
}

async fn write_file_handler(
    State(state): State<EditorState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<RepoQuery>,
    axum::extract::Path(rel_path): axum::extract::Path<String>,
    Json(body): Json<FileBody>,
) -> Response {
    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok());
    if let Err(e) = validate_builder_session(&state.portal_socket, cookie).await {
        return e;
    }

    if body.content.len() > MAX_FILE_SIZE {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("file exceeds maximum size of {} bytes", MAX_FILE_SIZE),
        );
    }

    let workspace = match resolve_workspace(&state.world_path, &query.repo) {
        Ok(ws) => ws,
        Err(e) => return e,
    };

    let file_path = match resolve_file_path(&workspace, &rel_path) {
        Ok(p) => p,
        Err(e) => return e,
    };

    if !file_path.exists() {
        return error_response(StatusCode::NOT_FOUND, "file not found");
    }

    match std::fs::write(&file_path, &body.content) {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write failed: {}", e),
        ),
    }
}

async fn create_file_handler(
    State(state): State<EditorState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<RepoQuery>,
    axum::extract::Path(rel_path): axum::extract::Path<String>,
    Json(body): Json<FileBody>,
) -> Response {
    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok());
    if let Err(e) = validate_builder_session(&state.portal_socket, cookie).await {
        return e;
    }

    if body.content.len() > MAX_FILE_SIZE {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("file exceeds maximum size of {} bytes", MAX_FILE_SIZE),
        );
    }

    let workspace = match resolve_workspace(&state.world_path, &query.repo) {
        Ok(ws) => ws,
        Err(e) => return e,
    };

    let file_path = match resolve_file_path(&workspace, &rel_path) {
        Ok(p) => p,
        Err(e) => return e,
    };

    if file_path.exists() {
        return error_response(StatusCode::CONFLICT, "file already exists");
    }

    // Ensure parent directories exist
    if let Some(parent) = file_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create directories: {}", e),
            );
        }
    }

    match std::fs::write(&file_path, &body.content) {
        Ok(()) => (StatusCode::CREATED, Json(serde_json::json!({ "status": "created" }))).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write failed: {}", e),
        ),
    }
}

async fn delete_file_handler(
    State(state): State<EditorState>,
    headers: axum::http::HeaderMap,
    Query(query): Query<RepoQuery>,
    axum::extract::Path(rel_path): axum::extract::Path<String>,
) -> Response {
    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok());
    if let Err(e) = validate_builder_session(&state.portal_socket, cookie).await {
        return e;
    }

    let workspace = match resolve_workspace(&state.world_path, &query.repo) {
        Ok(ws) => ws,
        Err(e) => return e,
    };

    let file_path = match resolve_file_path(&workspace, &rel_path) {
        Ok(p) => p,
        Err(e) => return e,
    };

    if !file_path.exists() {
        return error_response(StatusCode::NOT_FOUND, "file not found");
    }

    match std::fs::remove_file(&file_path) {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("delete failed: {}", e),
        ),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(ErrorBody { error: message.into() })).into_response()
}
