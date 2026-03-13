use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::{Html, Response};
use axum::routing::get;
use axum::Router;
use http_body_util::BodyExt;
use hyper::StatusCode;
use hyper_util::rt::TokioIo;
use tera::Tera;
use tokio::net::UnixStream;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::config::HttpConfig;
use crate::git::repo_manager::RepoManager;
use crate::git::workspace::Workspace;
use crate::mop_rpc::MopRpcClient;
use crate::persistence::ai_key_store::AiKeyStore;
use crate::persistence::player_store::PlayerStore;
use crate::server::{AreaTemplates, ServerCommand, TemplateRegistry};
use crate::web::ai::ai_routes;
use crate::web::build_log::BuildLog;
use crate::web::build_manager::BuildManager;
use crate::web::editor_files::editor_file_routes;
use crate::web::git_http::git_http_routes;
use crate::web::project::{build_log_routes, project_routes};
use crate::web::repos::repos_routes;
use crate::web::skills::SkillsService;

// ---------------------------------------------------------------------------
// AppState — shared state for all handlers
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct AppState {
    pub player_store: Arc<PlayerStore>,
    pub repo_manager: Arc<RepoManager>,
    pub workspace: Arc<Workspace>,
    pub templates: Arc<Tera>,
    pub ai_key_store: Option<Arc<AiKeyStore>>,
    pub skills_service: Option<Arc<SkillsService>>,
    pub http_client: reqwest::Client,
    pub portal_socket: String,
    pub anthropic_base_url: Option<String>,
    pub build_manager: Option<Arc<BuildManager>>,
    pub build_log: Arc<BuildLog>,
    pub mop_rpc: Option<MopRpcClient>,
    pub area_web_sockets: super::project::AreaWebSockets,
    pub area_templates: AreaTemplates,
    pub template_registry: TemplateRegistry,
    pub loaded_areas: std::sync::Arc<tokio::sync::RwLock<std::collections::HashSet<String>>>,
    pub server_commands: tokio::sync::mpsc::Sender<ServerCommand>,
}

// ---------------------------------------------------------------------------
// WebServer
// ---------------------------------------------------------------------------

pub struct WebServer {
    config: HttpConfig,
    state: AppState,
}

impl WebServer {
    pub fn new(config: HttpConfig, state: AppState) -> Self {
        Self { config, state }
    }

    /// Start the HTTP server, binding to the configured host and port.
    ///
    /// This function blocks until the server is shut down.
    pub async fn start(self) -> Result<()> {
        let world_path = self.state.workspace.world_path().to_path_buf();
        let build_cache_path = std::path::PathBuf::from(&self.config.build_cache_path);

        // Sub-routers that manage their own state (return Router<()>).
        // These must be added via nest_service since the main router uses AppState.
        let editor = editor_file_routes(
            world_path.clone(),
            self.state.mop_rpc.clone(),
            self.state.portal_socket.clone(),
        );
        let builder = build_log_routes(self.state.build_log.clone());
        let repos = repos_routes(
            self.state.repo_manager.clone(),
            self.state.workspace.clone(),
            self.state.area_templates.clone(),
            self.state.template_registry.clone(),
        );
        let project = project_routes(
            world_path,
            build_cache_path,
            self.state.build_log.clone(),
            self.state.mop_rpc.clone(),
            self.state.portal_socket.clone(),
            self.state.area_web_sockets.clone(),
        );

        let app = Router::new()
            .route("/", get(welcome_handler))
            .route("/api/areas/status", get(area_status_handler))
            .nest("/api/ai", ai_routes())
            .nest_service("/api/editor", editor)
            .nest_service("/api/builder", builder)
            .nest_service("/api/repos", repos)
            .nest("/git", git_http_routes())
            .nest_service("/project", project)
            .fallback(proxy_handler)
            .with_state(self.state)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            );

        let addr: SocketAddr = format!("{}:{}", self.config.host, self.config.port)
            .parse()
            .context("parsing HTTP bind address")?;

        tracing::info!(%addr, "HTTP server starting");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn area_status_handler(
    _user: crate::web::session::AuthUser,
    State(state): State<AppState>,
) -> axum::Json<serde_json::Value> {
    let loaded: Vec<String> = state.loaded_areas.read().await.iter().cloned().collect();
    let web_sockets: Vec<String> = state
        .area_web_sockets
        .read()
        .await
        .keys()
        .cloned()
        .collect();
    axum::Json(serde_json::json!({
        "loaded_areas": loaded,
        "web_sockets": web_sockets,
    }))
}

async fn welcome_handler(State(state): State<AppState>) -> Html<String> {
    let ctx = tera::Context::new();
    match state.templates.render("welcome.html", &ctx) {
        Ok(html) => Html(html),
        Err(e) => {
            let escaped = format!("{}", e)
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;");
            Html(format!("<h1>Template Error</h1><pre>{}</pre>", escaped))
        }
    }
}

// ---------------------------------------------------------------------------
// Reverse proxy fallback — forwards unmatched requests to the Ruby portal
// ---------------------------------------------------------------------------

/// Forwards an incoming request to the Ruby portal via a Unix domain socket
/// and returns the upstream response as-is.
async fn proxy_handler(State(state): State<AppState>, req: Request) -> Response {
    let socket_path = &state.portal_socket;

    // Connect to the portal Unix socket.
    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, path = %socket_path, "portal socket connect failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Portal unavailable: {}", e)))
                .unwrap();
        }
    };

    let io = TokioIo::new(stream);

    // Establish an HTTP/1.1 connection over the Unix socket.
    let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
        Ok(parts) => parts,
        Err(e) => {
            tracing::error!(error = %e, "portal handshake failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Handshake failed: {}", e)))
                .unwrap();
        }
    };

    // Drive the connection in a background task.
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!(error = %e, "portal proxy connection error");
        }
    });

    // Rebuild the request URI with only path + query (no scheme/authority
    // since we are speaking over a Unix socket).
    let path = req.uri().path();
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();
    let path_and_query = format!("{}{}", path, query);

    let method = req.method().clone();
    let headers = req.headers().clone();
    let body = req.into_body();

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(&path_and_query);

    // Forward all headers except Host.
    for (name, value) in headers.iter() {
        if name == axum::http::header::HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    // Set a Host header (required by HTTP/1.1) — the value doesn't matter
    // for a Unix socket but Rack expects it.
    builder = builder.header("host", "localhost");

    let upstream_req = builder.body(body).unwrap();

    // Send the request upstream.
    let upstream_resp = match sender.send_request(upstream_req).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(error = %e, "portal proxy request failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Proxy request failed: {}", e)))
                .unwrap();
        }
    };

    // Convert the hyper response into an axum response.
    let (parts, body) = upstream_resp.into_parts();
    let body = Body::new(body.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));

    let mut response = Response::builder().status(parts.status);
    for (name, value) in parts.headers.iter() {
        response = response.header(name, value);
    }

    response.body(body).unwrap()
}

// ---------------------------------------------------------------------------
// Template initialization
// ---------------------------------------------------------------------------

/// Initialize Tera templates from embedded strings.
///
/// Templates are compiled into the binary via `include_str!` so no filesystem
/// access is needed at runtime.
pub fn init_templates() -> Result<Tera> {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        ("layout.html", include_str!("templates/layout.html")),
        ("welcome.html", include_str!("templates/welcome.html")),
    ])?;
    Ok(tera)
}
