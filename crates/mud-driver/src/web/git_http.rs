use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::git::repo_manager::{AccessLevel, RepoManager};
use crate::persistence::player_store::{AuthResult, PlayerStore};
use crate::web::build_manager::BuildManager;
use crate::web::server::AppState;

// ---------------------------------------------------------------------------
// Route construction
// ---------------------------------------------------------------------------

/// Build the router for smart HTTP git protocol endpoints.
///
/// These routes implement the minimum subset of git's smart HTTP protocol
/// required for `git clone`, `git fetch`, and `git push` over HTTP(S).
///
/// Meant to be nested under `/git` in the main application:
/// ```rust,no_run
/// # use mud_driver::web::git_http::git_http_routes;
/// # use mud_driver::web::server::AppState;
/// # use axum::Router;
/// let app: Router<AppState> = Router::new()
///     .nest("/git", git_http_routes());
/// ```
pub fn git_http_routes() -> Router<AppState> {
    Router::new()
        .route("/{ns}/{repo}/info/refs", get(info_refs_handler))
        .route("/{ns}/{repo}/git-upload-pack", post(upload_pack_handler))
        .route("/{ns}/{repo}/git-receive-pack", post(receive_pack_handler))
}

// ---------------------------------------------------------------------------
// Pkt-line helpers
// ---------------------------------------------------------------------------

/// Encode a string as a git pkt-line (4 hex-digit length prefix + payload).
fn pkt_line(data: &str) -> Vec<u8> {
    let len = data.len() + 4;
    format!("{:04x}{}", len, data).into_bytes()
}

/// The special flush packet `0000` that terminates a pkt-line sequence.
fn flush_pkt() -> Vec<u8> {
    b"0000".to_vec()
}

// ---------------------------------------------------------------------------
// Basic auth extraction
// ---------------------------------------------------------------------------

/// Credentials extracted from an HTTP Basic `Authorization` header.
struct BasicCredentials {
    username: String,
    password: String,
}

/// Extract HTTP Basic credentials from the request headers.
///
/// Returns `None` if the `Authorization` header is missing or malformed.
fn extract_basic_auth(headers: &HeaderMap) -> Option<BasicCredentials> {
    let auth_value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = auth_value.strip_prefix("Basic ")?;
    let decoded =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok()?;
    let decoded_str = String::from_utf8(decoded).ok()?;
    let (username, password) = decoded_str.split_once(':')?;
    Some(BasicCredentials {
        username: username.to_string(),
        password: password.to_string(),
    })
}

/// Authenticate the user via HTTP Basic auth.
///
/// Supports two modes:
/// - Password auth: `player_store.authenticate(user, pass)`
/// - PAT auth: if the password starts with `"mud_"`, uses
///   `player_store.authenticate_token(user, pass)` instead.
///
/// Returns the authenticated username on success, or an HTTP error response
/// on failure (401 for missing/invalid credentials, with `WWW-Authenticate`
/// header to prompt git clients).
async fn authenticate(
    headers: &HeaderMap,
    player_store: &Arc<PlayerStore>,
) -> Result<String, Response> {
    let creds = extract_basic_auth(headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"MUD Git\"")],
            "Authentication required",
        )
            .into_response()
    })?;

    let result = if creds.password.starts_with("mud_") {
        player_store
            .authenticate_token(&creds.username, &creds.password)
            .await
    } else {
        player_store
            .authenticate(&creds.username, &creds.password)
            .await
    };

    match result {
        Ok(AuthResult::Success(_)) => Ok(creds.username),
        Ok(AuthResult::WrongPassword | AuthResult::NotFound) => Err((
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"MUD Git\"")],
            "Invalid credentials",
        )
            .into_response()),
        Err(e) => {
            tracing::error!(error = %e, "authentication error during git HTTP access");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// Check that the user has the required access level for the repository.
///
/// Returns 404 if the repo does not exist, 403 if the user lacks permission.
#[allow(clippy::result_large_err)]
fn check_acl(
    repo_manager: &Arc<RepoManager>,
    username: &str,
    ns: &str,
    name: &str,
    level: &AccessLevel,
) -> Result<(), Response> {
    if !repo_manager.repo_exists(ns, name) {
        return Err(StatusCode::NOT_FOUND.into_response());
    }

    if !repo_manager.can_access(username, ns, name, level) {
        return Err(StatusCode::FORBIDDEN.into_response());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Query parameter
// ---------------------------------------------------------------------------

/// Query parameters for the `info/refs` endpoint.
#[derive(serde::Deserialize)]
struct InfoRefsQuery {
    service: String,
}

// ---------------------------------------------------------------------------
// Path parameter
// ---------------------------------------------------------------------------

/// Path parameters extracted from `/:ns/:repo/...`.
///
/// The `repo` field may include a `.git` suffix (e.g. `"village.git"`),
/// which is stripped by `repo_name()` to yield just the area name.
#[derive(serde::Deserialize)]
struct RepoPath {
    ns: String,
    repo: String,
}

impl RepoPath {
    /// Return the area name, stripping any `.git` suffix.
    fn repo_name(&self) -> &str {
        self.repo.strip_suffix(".git").unwrap_or(&self.repo)
    }
}

// ---------------------------------------------------------------------------
// GET /info/refs — ref advertisement
// ---------------------------------------------------------------------------

/// Handle `GET /:ns/:name.git/info/refs?service=git-upload-pack|git-receive-pack`.
///
/// This is the first request a git client makes when cloning or pushing.
/// It returns a list of all references in the repository formatted as
/// pkt-line data, preceded by a service header.
async fn info_refs_handler(
    State(state): State<AppState>,
    Path(repo_path): Path<RepoPath>,
    Query(query): Query<InfoRefsQuery>,
    headers: HeaderMap,
) -> Result<Response, Response> {
    let ns = &repo_path.ns;
    let name = repo_path.repo_name();

    // Validate the service parameter.
    let service = &query.service;
    if service != "git-upload-pack" && service != "git-receive-pack" {
        return Err((StatusCode::BAD_REQUEST, "Invalid service").into_response());
    }

    // Authenticate.
    let username = authenticate(&headers, &state.player_store).await?;

    // Determine required access level.
    let required_level = if service == "git-receive-pack" {
        AccessLevel::ReadWrite
    } else {
        AccessLevel::ReadOnly
    };
    check_acl(&state.repo_manager, &username, ns, name, &required_level)?;

    // Open the bare repository.
    let disk_path = state.repo_manager.repo_path(ns, name);
    let repo = git2::Repository::open_bare(&disk_path).map_err(|e| {
        tracing::error!(error = %e, ns, name, "failed to open bare repo");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    // Build the pkt-line response.
    let mut body: Vec<u8> = Vec::new();

    // Service header line (inside its own pkt-line).
    body.extend(pkt_line(&format!("# service={}\n", service)));
    body.extend(flush_pkt());

    // Collect references.
    let references = repo.references().map_err(|e| {
        tracing::error!(error = %e, "failed to iterate references");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    let mut ref_lines: Vec<(String, String)> = Vec::new();

    // HEAD — resolve to the target OID if available.
    if let Ok(head) = repo.head() {
        if let Some(oid) = head.target() {
            ref_lines.push((oid.to_string(), "HEAD".to_string()));
        }
    }

    for reference in references {
        let reference = match reference {
            Ok(r) => r,
            Err(_) => continue,
        };
        let ref_name = match reference.name() {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Resolve to direct OID (peel symrefs).
        let oid = match reference.target().or_else(|| {
            reference
                .resolve()
                .ok()
                .and_then(|resolved| resolved.target())
        }) {
            Some(o) => o,
            None => continue,
        };
        ref_lines.push((oid.to_string(), ref_name));
    }

    // Format each ref as a pkt-line: "<oid> <refname>\n".
    // The first line carries the capabilities after a NUL byte.
    let capabilities = "report-status delete-refs side-band-64k ofs-delta";
    for (i, (oid, refname)) in ref_lines.iter().enumerate() {
        let line = if i == 0 {
            format!("{} {}\0{}\n", oid, refname, capabilities)
        } else {
            format!("{} {}\n", oid, refname)
        };
        body.extend(pkt_line(&line));
    }

    body.extend(flush_pkt());

    let content_type = format!("application/x-{}-advertisement", service);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        body,
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// POST /git-upload-pack — fetch
// ---------------------------------------------------------------------------

/// Handle `POST /:ns/:name.git/git-upload-pack`.
///
/// Spawns `git upload-pack --stateless-rpc <repo-path>` as a child process,
/// pipes the request body to its stdin, and streams stdout back as the
/// response body.
async fn upload_pack_handler(
    State(state): State<AppState>,
    Path(repo_path): Path<RepoPath>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Response> {
    run_service(
        &state,
        &repo_path.ns,
        repo_path.repo_name(),
        "git-upload-pack",
        "upload-pack",
        &AccessLevel::ReadOnly,
        &headers,
        body,
    )
    .await
}

// ---------------------------------------------------------------------------
// POST /git-receive-pack — push
// ---------------------------------------------------------------------------

/// Handle `POST /:ns/:name.git/git-receive-pack`.
///
/// Spawns `git receive-pack --stateless-rpc <repo-path>` as a child process,
/// pipes the request body to its stdin, and streams stdout back as the
/// response body.
async fn receive_pack_handler(
    State(state): State<AppState>,
    Path(repo_path): Path<RepoPath>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Response> {
    run_service(
        &state,
        &repo_path.ns,
        repo_path.repo_name(),
        "git-receive-pack",
        "receive-pack",
        &AccessLevel::ReadWrite,
        &headers,
        body,
    )
    .await
}

// ---------------------------------------------------------------------------
// Shared service execution
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn pkt_line_encodes_service_header() {
        let line = pkt_line("# service=git-upload-pack\n");
        let s = String::from_utf8(line).unwrap();
        // 4 hex length + payload
        assert!(s.starts_with("001e"));
        assert!(s.contains("# service=git-upload-pack\n"));
        // Total length = 4 (prefix) + 26 (payload) = 30 = 0x1e
        assert_eq!(s.len(), 30);
    }

    #[test]
    fn pkt_line_encodes_short_string() {
        let line = pkt_line("hi\n");
        let s = String::from_utf8(line).unwrap();
        // "hi\n" = 3 bytes, + 4 prefix = 7 = 0x0007
        assert_eq!(s, "0007hi\n");
    }

    #[test]
    fn pkt_line_encodes_empty_string() {
        let line = pkt_line("");
        let s = String::from_utf8(line).unwrap();
        // Empty payload, just the 4-byte prefix: 0004
        assert_eq!(s, "0004");
    }

    #[test]
    fn flush_pkt_returns_0000() {
        let pkt = flush_pkt();
        assert_eq!(pkt, b"0000");
    }

    #[test]
    fn extract_basic_auth_valid_credentials() {
        let mut headers = HeaderMap::new();
        // "alice:secret123" base64 = "YWxpY2U6c2VjcmV0MTIz"
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic YWxpY2U6c2VjcmV0MTIz"),
        );
        let creds = extract_basic_auth(&headers).unwrap();
        assert_eq!(creds.username, "alice");
        assert_eq!(creds.password, "secret123");
    }

    #[test]
    fn extract_basic_auth_missing_header() {
        let headers = HeaderMap::new();
        assert!(extract_basic_auth(&headers).is_none());
    }

    #[test]
    fn extract_basic_auth_non_basic_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer some-token"),
        );
        assert!(extract_basic_auth(&headers).is_none());
    }

    #[test]
    fn extract_basic_auth_invalid_base64() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic !!!invalid!!!"),
        );
        assert!(extract_basic_auth(&headers).is_none());
    }

    #[test]
    fn extract_basic_auth_no_colon_in_decoded() {
        let mut headers = HeaderMap::new();
        // "nocolon" base64 = "bm9jb2xvbg=="
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic bm9jb2xvbg=="),
        );
        assert!(extract_basic_auth(&headers).is_none());
    }

    #[test]
    fn extract_basic_auth_empty_password() {
        let mut headers = HeaderMap::new();
        // "user:" base64 = "dXNlcjo="
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcjo="),
        );
        let creds = extract_basic_auth(&headers).unwrap();
        assert_eq!(creds.username, "user");
        assert_eq!(creds.password, "");
    }

    #[test]
    fn extract_basic_auth_password_with_colon() {
        let mut headers = HeaderMap::new();
        // "user:pass:word" base64 = "dXNlcjpwYXNzOndvcmQ="
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcjpwYXNzOndvcmQ="),
        );
        let creds = extract_basic_auth(&headers).unwrap();
        assert_eq!(creds.username, "user");
        // split_once means password keeps the rest including colons
        assert_eq!(creds.password, "pass:word");
    }

    #[test]
    fn pkt_line_ref_with_capabilities() {
        let cap = "report-status delete-refs";
        let line = format!("abc123 HEAD\0{}\n", cap);
        let encoded = pkt_line(&line);
        let s = String::from_utf8(encoded).unwrap();
        assert!(s.contains("abc123 HEAD"));
        assert!(s.contains(cap));
        // The NUL byte should be in there
        assert!(s.contains('\0'));
    }
}

/// Execute a git service (`upload-pack` or `receive-pack`) as a child process.
///
/// The `service_name` is the full service name (e.g. `"git-upload-pack"`)
/// used for content-type headers. The `git_command` is the subcommand passed
/// to `git` (e.g. `"upload-pack"`).
#[allow(clippy::too_many_arguments)]
async fn run_service(
    state: &AppState,
    ns: &str,
    name: &str,
    service_name: &str,
    git_command: &str,
    required_level: &AccessLevel,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response, Response> {
    // Authenticate.
    let username = authenticate(headers, &state.player_store).await?;

    // ACL check.
    check_acl(&state.repo_manager, &username, ns, name, required_level)?;

    // Resolve repo path on disk.
    let repo_disk_path = state.repo_manager.repo_path(ns, name);
    let repo_str = repo_disk_path.to_string_lossy().to_string();

    // Spawn git process.
    let mut child = Command::new("git")
        .arg(git_command)
        .arg("--stateless-rpc")
        .arg(&repo_str)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            tracing::error!(error = %e, "failed to spawn git {}", git_command);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;

    // Write request body to stdin.
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = stdin.write_all(&body).await {
            tracing::error!(error = %e, "failed to write to git stdin");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
        // Drop stdin to close it, signaling EOF to the child.
        drop(stdin);
    }

    // Read stdout into a buffer.
    let mut stdout_data = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        if let Err(e) = stdout.read_to_end(&mut stdout_data).await {
            tracing::error!(error = %e, "failed to read git stdout");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    // Wait for the child process to finish.
    let status = child.wait().await.map_err(|e| {
        tracing::error!(error = %e, "failed to wait for git process");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    if !status.success() {
        // Read stderr for diagnostics.
        tracing::warn!(
            ns,
            name,
            service = service_name,
            exit_code = status.code(),
            "git process exited with non-zero status"
        );
        // Still return the stdout — git clients expect the error payload there.
    }

    // After a successful receive-pack (push), trigger SPA builds if applicable.
    // We don't know which branch was pushed, so rebuild both production and @dev.
    if service_name == "git-receive-pack" && status.success() {
        if let Some(ref build_manager) = state.build_manager {
            // Production (main branch)
            let _ = state.workspace.pull(ns, name, "main");
            let area_path = state.workspace.workspace_path(ns, name);
            if BuildManager::is_spa(&area_path) {
                let base_url = format!("/project/{ns}/{name}/");
                let area_key = format!("{ns}/{name}");
                build_manager.trigger_build(area_key, area_path, base_url);
            }

            // Development (@dev branch)
            let _ = state.workspace.pull(ns, name, "develop");
            let dev_path = state.workspace.dev_path(ns, name);
            if BuildManager::is_spa(&dev_path) {
                let base_url = format!("/project/{ns}/{name}@dev/");
                let area_key = format!("{ns}/{name}@dev");
                build_manager.trigger_build(area_key, dev_path, base_url);
            }
        }
    }

    let content_type = format!("application/x-{}-result", service_name);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        stdout_data,
    )
        .into_response())
}
