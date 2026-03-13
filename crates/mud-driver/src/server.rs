use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use mud_core::types::AreaId;
use mud_mop::message::{AdapterMessage, DriverMessage, Value};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Thread-safe, shared area template registry.
/// Outer key is template name (e.g. "default", "kotlin:ktor"), inner keys
/// are file paths with `{{namespace}}` / `{{area_name}}` placeholders.
pub type AreaTemplates = Arc<RwLock<HashMap<String, HashMap<String, String>>>>;

/// Thread-safe, shared registry of template repos discovered under `system/`.
pub type TemplateRegistry = Arc<RwLock<HashMap<String, RegisteredTemplate>>>;

#[derive(Debug, Clone)]
pub struct RegisteredTemplate {
    pub name: String,
    pub repo_namespace: String,
    pub repo_name: String,
    pub path: PathBuf,
    pub metadata: TemplateMetadata,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemplateMetadata {
    pub name: String,
    pub kind: String,
    pub language: String,
    #[serde(default)]
    pub framework: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub stdlib_compatible: Option<bool>,
}

use crate::config::Config;
use crate::git::merge_request_manager::MergeRequestManager;
use crate::git::repo_manager::RepoManager;
use crate::git::workspace::Workspace;
use crate::mop_rpc::{MopRequest, MopRpcClient};
use crate::persistence::ai_key_store::AiKeyStore;
use crate::persistence::credential_encryptor::CredentialEncryptor;
use crate::persistence::database_manager::DatabaseManager;
use crate::persistence::player_store::{AuthResult, PlayerStore};
use crate::runtime::adapter_manager::{AdapterManager, SourcedMessage};
use crate::runtime::object_broker::ObjectBroker;
use crate::runtime::state_store::StateStore;
use crate::runtime::version_tree::VersionTree;
use crate::ssh::handler::SshCommand;
use crate::ssh::server::start_ssh_server;
use crate::web::build_log::BuildLog;
use crate::web::build_manager::BuildManager;
use crate::web::server::{init_templates, AppState, WebServer};
use crate::web::skills::SkillsService;

#[derive(Debug, Clone)]
pub enum ServerCommand {
    RepoUpdated { namespace: String, name: String },
}

// ---------------------------------------------------------------------------
// SessionState — tracks active player sessions
// ---------------------------------------------------------------------------

struct SessionState {
    output_senders: HashMap<u64, mpsc::Sender<String>>,
    next_session_id: u64,
}

impl SessionState {
    fn new() -> Self {
        Self {
            output_senders: HashMap::new(),
            next_session_id: 1,
        }
    }

    /// Assign a new session ID and store the output sender.
    fn allocate_session(&mut self, tx: mpsc::Sender<String>) -> u64 {
        let id = self.next_session_id;
        self.next_session_id += 1;
        self.output_senders.insert(id, tx);
        id
    }

    /// Remove a session's output sender.
    fn remove_session(&mut self, id: u64) {
        self.output_senders.remove(&id);
    }

    /// Try to send output text to a session. Ignores errors if the channel
    /// is full or closed (the session may have disconnected).
    fn send_output(&self, session_id: u64, text: String) {
        if let Some(tx) = self.output_senders.get(&session_id) {
            if let Err(e) = tx.try_send(text) {
                warn!(session_id, %e, "failed to send output to session");
            }
        } else {
            warn!(session_id, "no output channel for session");
        }
    }
}

// ---------------------------------------------------------------------------
// Server — main orchestrator
// ---------------------------------------------------------------------------

/// The main server struct that coordinates config, adapter management,
/// session state, and area loading.
pub struct Server {
    config: Config,
    adapter_manager: AdapterManager,
    sessions: SessionState,
    /// Language identifiers returned by adapter handshakes (e.g. "ruby", "kotlin").
    /// Includes both primary and alias languages for routing.
    adapter_languages: Vec<String>,
    /// Primary language per adapter (one per process), used for broadcast
    /// operations like `send_configure` to avoid sending duplicates when an
    /// adapter handles multiple languages.
    adapter_primary_languages: Vec<String>,
    /// Database manager, initialized when `database.admin_password` is configured.
    #[allow(dead_code)]
    db_manager: Option<DatabaseManager>,
    /// Credential encryptor for AES-256-GCM encryption of stored secrets.
    encryptor: Option<Arc<CredentialEncryptor>>,
    /// Player store for account/character/session operations.
    player_store: Option<Arc<PlayerStore>>,
    /// Git repository manager for bare repo + ACL management.
    repo_manager: Option<Arc<RepoManager>>,
    /// Workspace manager for working directory operations.
    workspace: Option<Arc<Workspace>>,
    /// Merge request manager for MR lifecycle operations.
    merge_request_manager: Option<Arc<MergeRequestManager>>,
    /// Area template registry, shared with the web layer for the repos API.
    area_templates: AreaTemplates,
    /// System template registry backed by `system/template_*` repos.
    template_registry: TemplateRegistry,
    /// Per-area web socket paths for API proxying in SPA mode.
    /// Shared with the HTTP server for live routing updates.
    area_web_sockets: crate::web::project::AreaWebSockets,
    /// Set of area keys that have reported successful loading.
    /// Shared with the web layer for the area status API.
    loaded_areas: Arc<RwLock<std::collections::HashSet<String>>>,
    /// Background task handle for the HTTP web server.
    #[allow(dead_code)]
    web_handle: Option<JoinHandle<()>>,
    /// Path to the master JSONL log file (`{world}/.mud/driver.log`).
    master_log_path: Option<std::path::PathBuf>,
    /// Build log for tracking SPA build output.
    build_log: Arc<BuildLog>,
    /// Build manager for triggering SPA builds (requires workspace).
    build_manager: Option<Arc<BuildManager>>,
    /// Receiving end of the MOP RPC channel (web handlers submit requests here).
    mop_rpc_rx: Option<mpsc::Receiver<MopRequest>>,
    /// MOP RPC client for passing to web state; created together with `mop_rpc_rx`.
    mop_rpc_client: Option<MopRpcClient>,
    /// Pending MOP RPC calls: request_id -> oneshot sender for the response.
    pending_rpc: HashMap<u64, oneshot::Sender<Result<Value, String>>>,
    /// Counter for assigning unique request IDs to MOP RPC calls.
    next_rpc_id: u64,
    /// Internal driver command receiver used by web handlers to notify the main loop.
    server_command_rx: Option<mpsc::Receiver<ServerCommand>>,
    /// Sender for internal driver commands.
    server_command_tx: mpsc::Sender<ServerCommand>,
    /// Runtime dependency graph used for selective reload propagation.
    version_tree: Arc<RwLock<VersionTree>>,
    /// Driver-owned object state for cache invalidation and future reload precision.
    state_store: Arc<RwLock<StateStore>>,
    /// Cached call broker; invalidated on affected program reloads.
    object_broker: Arc<RwLock<ObjectBroker>>,
}

impl Server {
    /// Create a new server with the given configuration.
    ///
    /// The Unix socket path is derived from the current process ID to avoid
    /// conflicts when multiple instances run on the same machine.
    pub fn new(config: Config) -> Self {
        let socket_path = format!("/tmp/mud-driver-{}.sock", std::process::id());
        Self::new_with_socket_path(config, socket_path.into())
    }

    /// Create a new server with an explicit socket path.
    ///
    /// Useful for tests that need unique socket paths to avoid conflicts
    /// when running in parallel.
    pub fn new_with_socket_path(config: Config, socket_path: std::path::PathBuf) -> Self {
        let adapter_manager = AdapterManager::new(socket_path);
        let master_log_path = Some(config.world.resolved_path().join(".mud").join("driver.log"));
        let build_log = Arc::new(BuildLog::new(200));
        let build_cache_path = std::path::PathBuf::from(&config.http.build_cache_path);
        let build_manager = Some(Arc::new(BuildManager::new(
            Arc::clone(&build_log),
            build_cache_path,
        )));

        // Create the MOP RPC channel for web-handler-to-adapter communication.
        let (mop_rpc_tx, mop_rpc_rx) = mpsc::channel::<MopRequest>(32);
        let mop_rpc_client = MopRpcClient::new(mop_rpc_tx);
        let (server_command_tx, server_command_rx) = mpsc::channel::<ServerCommand>(64);

        Self {
            config,
            adapter_manager,
            sessions: SessionState::new(),
            adapter_languages: Vec::new(),
            adapter_primary_languages: Vec::new(),
            db_manager: None,
            encryptor: None,
            player_store: None,
            repo_manager: None,
            workspace: None,
            merge_request_manager: None,
            area_templates: Arc::new(RwLock::new(HashMap::new())),
            template_registry: Arc::new(RwLock::new(HashMap::new())),
            area_web_sockets: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            loaded_areas: Arc::new(RwLock::new(std::collections::HashSet::new())),
            web_handle: None,
            master_log_path,
            build_log,
            build_manager,
            mop_rpc_rx: Some(mop_rpc_rx),
            mop_rpc_client: Some(mop_rpc_client),
            pending_rpc: HashMap::new(),
            next_rpc_id: 1_000_000, // Start high to avoid collisions with adapter request IDs
            server_command_rx: Some(server_command_rx),
            server_command_tx,
            version_tree: Arc::new(RwLock::new(VersionTree::new())),
            state_store: Arc::new(RwLock::new(StateStore::new())),
            object_broker: Arc::new(RwLock::new(ObjectBroker::new())),
        }
    }

    /// Boot the server: start the adapter manager, accept an adapter
    /// connection, start the SSH server, load areas, then enter the
    /// unified event loop.
    pub async fn boot(&mut self) -> Result<()> {
        info!(server_name = %self.config.server_name, "Starting server");

        let listener = self
            .adapter_manager
            .start(&self.config)
            .await
            .context("starting adapter manager")?;

        // Count how many adapters are enabled so we know how many to wait for.
        let expected_adapters = enabled_adapter_count(&self.config);

        info!(expected_adapters, "Waiting for adapters (30s timeout)...");

        for i in 0..expected_adapters {
            let (language, additional) = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                self.adapter_manager.accept_connection(&listener),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "timed out waiting for adapter {}/{} to connect (30s). \
                 Check that the adapter binary exists and is configured.",
                    i + 1,
                    expected_adapters,
                )
            })?
            .context("accepting adapter connection")?;

            info!(
                language,
                adapter = i + 1,
                total = expected_adapters,
                "Adapter connected"
            );
            self.adapter_primary_languages.push(language.clone());
            self.adapter_languages.push(language);
            for lang in additional {
                if !self.adapter_languages.contains(&lang) {
                    info!(alias = %lang, "Registered additional adapter language");
                    self.adapter_languages.push(lang);
                }
            }
        }

        // -----------------------------------------------------------------
        // Scan disk-based templates so they're always available, even when
        // an adapter isn't running.  Adapters that *are* connected already
        // sent their templates during the handshake above, so we only
        // insert templates that aren't already registered.
        // -----------------------------------------------------------------
        if !self.system_templates_exist() {
            self.scan_disk_templates().await;
        }

        // -----------------------------------------------------------------
        // Database setup (optional — requires admin_password)
        // -----------------------------------------------------------------
        self.setup_database().await?;
        self.refresh_template_registry().await?;

        // -----------------------------------------------------------------
        // Web server (optional — requires http.enabled)
        // -----------------------------------------------------------------
        if self.config.http.enabled {
            if let (Some(ps), Some(rm), Some(ws)) =
                (&self.player_store, &self.repo_manager, &self.workspace)
            {
                let templates = Arc::new(init_templates().context("initializing web templates")?);

                // Build AiKeyStore — reuse encryptor from database setup.
                let ai_key_store =
                    if let (Some(db_mgr), Some(encryptor)) = (&self.db_manager, &self.encryptor) {
                        info!("AI key store initialized");
                        Some(Arc::new(AiKeyStore::new(
                            db_mgr.driver_pool().clone(),
                            Arc::clone(encryptor),
                        )))
                    } else {
                        info!("AI key store not configured (no database manager or encryptor)");
                        None
                    };

                // Build SkillsService from AI config
                let skills_service = match SkillsService::new(&self.config.ai).await {
                    Ok(svc) => Some(Arc::new(svc)),
                    Err(e) => {
                        warn!(error = %e, "failed to initialize skills service");
                        None
                    }
                };

                let state = AppState {
                    player_store: Arc::clone(ps),
                    repo_manager: Arc::clone(rm),
                    workspace: Arc::clone(ws),
                    templates,
                    ai_key_store,
                    skills_service,
                    http_client: reqwest::Client::new(),
                    portal_socket: self.config.http.portal_socket.clone(),
                    anthropic_base_url: self.config.ai.anthropic_base_url.clone(),
                    build_manager: self.build_manager.clone(),
                    build_log: Arc::clone(&self.build_log),
                    mop_rpc: self.mop_rpc_client.clone(),
                    area_web_sockets: Arc::clone(&self.area_web_sockets),
                    area_templates: Arc::clone(&self.area_templates),
                    template_registry: Arc::clone(&self.template_registry),
                    loaded_areas: Arc::clone(&self.loaded_areas),
                    server_commands: self.server_command_tx.clone(),
                };

                let web_server = WebServer::new(self.config.http.clone(), state);
                let handle = tokio::spawn(async move {
                    if let Err(e) = web_server.start().await {
                        error!(error = %e, "HTTP server failed");
                    }
                });

                self.web_handle = Some(handle);

                info!(
                    host = %self.config.http.host,
                    port = self.config.http.port,
                    "HTTP server started"
                );
            } else {
                warn!(
                    "HTTP server enabled but database is not configured — \
                     skipping web server (requires database.admin_password)"
                );
            }
        } else {
            info!("HTTP server disabled");
        }

        // -----------------------------------------------------------------
        // Area loading
        // -----------------------------------------------------------------
        self.load_areas().await?;

        // -----------------------------------------------------------------
        // SSH server
        // -----------------------------------------------------------------
        let (ssh_cmd_tx, ssh_cmd_rx) = mpsc::channel::<SshCommand>(256);
        let ssh_config = self.config.ssh.clone();
        let ssh_player_store = self.player_store.clone();
        tokio::spawn(async move {
            if let Err(e) = start_ssh_server(&ssh_config, ssh_cmd_tx, ssh_player_store).await {
                error!(error = %e, "SSH server failed");
            }
        });
        info!(
            host = %self.config.ssh.host,
            port = self.config.ssh.port,
            "SSH server started"
        );

        info!("Server ready");

        self.run_event_loop(ssh_cmd_rx).await
    }

    /// Scan the world directory for areas and send LoadArea messages to the
    /// connected adapter for each one.
    pub async fn load_areas(&self) -> Result<()> {
        if self.adapter_languages.is_empty() {
            return Err(anyhow::anyhow!("no adapter connected — cannot load areas"));
        }

        self.ensure_runtime_dependency_roots().await;
        let areas = discover_areas(&self.config.world.resolved_path().to_string_lossy())?;

        for (area_id, area_path) in &areas {
            let language = self.language_for_area(area_path);
            self.register_area_runtime_dependencies(area_id, area_path, &language)
                .await;
            info!(%area_id, path = %area_path, %language, "Loading area");

            // Provision per-area database (idempotent)
            let db_url = if let Some(ref db_mgr) = self.db_manager {
                if let Err(e) = db_mgr
                    .provision_area_db(&area_id.namespace, &area_id.name)
                    .await
                {
                    warn!(%area_id, %e, "Failed to provision area database");
                    None
                } else {
                    match db_mgr
                        .get_area_db_url(&area_id.namespace, &area_id.name)
                        .await
                    {
                        Ok(url) => url,
                        Err(e) => {
                            warn!(%area_id, %e, "Failed to get area database URL");
                            None
                        }
                    }
                }
            } else {
                None
            };

            self.adapter_manager
                .send_to(
                    &language,
                    DriverMessage::LoadArea {
                        area_id: area_id.clone(),
                        path: area_path.clone(),
                        db_url,
                    },
                )
                .await
                .context("sending LoadArea to adapter")?;
        }

        Ok(())
    }

    /// Create a new player session, returning the session ID and a receiver
    /// for output text destined for the player.
    pub fn create_session(&mut self) -> (u64, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(64);
        let session_id = self.sessions.allocate_session(tx);
        (session_id, rx)
    }

    /// Return the primary adapter language. Ruby is always preferred since it
    /// runs the game engine (sessions, area loading, portal). Other adapters
    /// (e.g. JVM/kotlin) are supplementary and handle specific area types.
    fn primary_language(&self) -> &str {
        self.adapter_languages
            .iter()
            .find(|l| l.as_str() == "ruby")
            .map(|s| s.as_str())
            .or_else(|| self.adapter_languages.first().map(|s| s.as_str()))
            .unwrap_or("ruby")
    }

    /// Determine the adapter language for a given area by reading its `mud.yaml`.
    ///
    /// If `mud.yaml` exists and specifies `language: lpc`, the area belongs to
    /// the "lpc" adapter. If it specifies a JVM framework (anything other than
    /// `none`), the area belongs to the "kotlin" adapter. Otherwise it defaults
    /// to the primary language (Ruby).
    fn language_for_area(&self, area_path: &str) -> String {
        let yaml_path = std::path::Path::new(area_path).join("mud.yaml");
        match std::fs::read_to_string(&yaml_path) {
            Ok(contents) => {
                if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&contents) {
                    // Check for explicit language field first.
                    if let Some(language) = yaml.get("language").and_then(|v| v.as_str()) {
                        if language == "lpc" && self.adapter_languages.iter().any(|l| l == "lpc") {
                            info!(path = %area_path, %language, "Routing area to lpc adapter");
                            return "lpc".to_string();
                        }
                        if language == "rust" && self.adapter_languages.iter().any(|l| l == "rust")
                        {
                            info!(path = %area_path, %language, "Routing area to rust adapter");
                            return "rust".to_string();
                        }
                    }

                    // Check for framework field.
                    if let Some(framework) = yaml.get("framework").and_then(|v| v.as_str()) {
                        if framework == "lpc" && self.adapter_languages.iter().any(|l| l == "lpc") {
                            info!(path = %area_path, %framework, "Routing area to lpc adapter");
                            return "lpc".to_string();
                        }
                        if framework != "none"
                            && self.adapter_languages.iter().any(|l| l == "kotlin")
                        {
                            info!(path = %area_path, %framework, "Routing area to kotlin adapter");
                            return "kotlin".to_string();
                        }
                        info!(path = %area_path, %framework, "Area uses ruby (framework=none or no matching adapter)");
                    }
                }
            }
            Err(e) => {
                info!(path = %yaml_path.display(), %e, "No mud.yaml found, defaulting to ruby");
            }
        }
        self.primary_language().to_string()
    }

    /// Notify the adapter that a player session has started.
    pub async fn session_start(&self, session_id: u64, username: String) -> Result<()> {
        self.adapter_manager
            .send_to(
                self.primary_language(),
                DriverMessage::SessionStart {
                    session_id,
                    username,
                },
            )
            .await
    }

    /// Forward a line of player input to the adapter.
    pub async fn session_input(&self, session_id: u64, line: String) -> Result<()> {
        self.adapter_manager
            .send_to(
                self.primary_language(),
                DriverMessage::SessionInput { session_id, line },
            )
            .await
    }

    /// End a player session: remove it from the session state and notify the
    /// adapter.
    pub async fn session_end(&mut self, session_id: u64) -> Result<()> {
        self.sessions.remove_session(session_id);
        self.adapter_manager
            .send_to(
                self.primary_language(),
                DriverMessage::SessionEnd { session_id },
            )
            .await
    }

    // -----------------------------------------------------------------
    // Test-friendly methods for component-level integration testing.
    // These expose individual boot phases so tests can drive the server
    // without needing SSH, PostgreSQL, or a real adapter binary.
    // -----------------------------------------------------------------

    /// Start the adapter manager's Unix listener. Returns the listener
    /// so the caller can drive `accept_adapter` separately.
    pub async fn start_adapter_manager(&mut self) -> Result<tokio::net::UnixListener> {
        self.adapter_manager
            .start(&self.config)
            .await
            .context("starting adapter manager")
    }

    /// Accept an adapter connection on the given listener, storing the
    /// adapter language(s) for subsequent session calls.
    pub async fn accept_adapter(&mut self, listener: &tokio::net::UnixListener) -> Result<String> {
        let (language, additional) = self
            .adapter_manager
            .accept_connection(listener)
            .await
            .context("accepting adapter connection")?;
        self.adapter_primary_languages.push(language.clone());
        self.adapter_languages.push(language.clone());
        for lang in additional {
            if !self.adapter_languages.contains(&lang) {
                self.adapter_languages.push(lang);
            }
        }
        Ok(language)
    }

    /// Return the Unix socket path the adapter manager is bound to.
    pub fn socket_path(&self) -> &std::path::Path {
        self.adapter_manager.socket_path()
    }

    /// Receive the next message from any connected adapter.
    pub async fn recv_adapter_message(&mut self) -> Option<SourcedMessage> {
        self.adapter_manager.recv().await
    }

    /// Set the player store (used in tests to inject a pre-configured store).
    pub fn set_player_store(&mut self, store: Arc<PlayerStore>) {
        self.player_store = Some(store);
    }

    /// Set the repo manager (used in tests to inject a pre-configured manager).
    pub fn set_repo_manager(&mut self, rm: Arc<RepoManager>) {
        self.repo_manager = Some(rm);
    }

    /// Set the workspace (used in tests to inject a pre-configured workspace).
    pub fn set_workspace(&mut self, ws: Arc<Workspace>) {
        self.workspace = Some(ws);
    }

    pub fn set_merge_request_manager(&mut self, mrm: Arc<MergeRequestManager>) {
        self.merge_request_manager = Some(mrm);
    }

    /// Send a Configure message to all connected adapters with the stdlib DB URL.
    /// Each adapter uses this to connect and run its own schema migrations.
    /// Uses primary languages only to avoid sending duplicates when an adapter
    /// handles multiple language aliases.
    pub async fn send_configure(&self, stdlib_db_url: String) -> Result<()> {
        for lang in &self.adapter_primary_languages {
            self.adapter_manager
                .send_to(
                    lang,
                    DriverMessage::Configure {
                        stdlib_db_url: stdlib_db_url.clone(),
                    },
                )
                .await
                .with_context(|| format!("sending configure to {lang} adapter"))?;
        }
        Ok(())
    }

    /// Send a LoadArea message to the appropriate adapter based on area type.
    pub async fn send_load_area(&self, area_id: AreaId, path: String) -> Result<()> {
        if !self.adapter_languages.is_empty() {
            let lang = self.language_for_area(&path);
            self.ensure_runtime_dependency_roots().await;
            self.register_area_runtime_dependencies(&area_id, &path, &lang)
                .await;
            let db_url = if let Some(ref db_mgr) = self.db_manager {
                if let Err(e) = db_mgr
                    .provision_area_db(&area_id.namespace, &area_id.name)
                    .await
                {
                    warn!(%area_id, %e, "Failed to provision area database");
                    None
                } else {
                    db_mgr
                        .get_area_db_url(&area_id.namespace, &area_id.name)
                        .await
                        .unwrap_or(None)
                }
            } else {
                None
            };

            self.adapter_manager
                .send_to(
                    &lang,
                    DriverMessage::LoadArea {
                        area_id,
                        path,
                        db_url,
                    },
                )
                .await
                .context("sending LoadArea to adapter")?;
        }
        Ok(())
    }

    /// Initialize the database, create PlayerStore/RepoManager/Workspace,
    /// and send the stdlib DB URL to the adapter so it can run migrations.
    ///
    /// This is the same initialization that `boot()` performs. Exposed
    /// publicly so that tests can replicate the production startup flow
    /// without calling `boot()` (which also starts SSH, HTTP, etc.).
    pub async fn setup_database(&mut self) -> Result<()> {
        if self.config.database.admin_password.is_none() {
            info!("No database password configured — skipping database setup");
            return Ok(());
        }

        info!("Initializing database...");

        let encryptor = {
            let key_hex = match &self.config.database.encryption_key {
                Some(k) => k.clone(),
                None => load_or_generate_encryption_key(&self.config.world.data_path)?,
            };
            let key_bytes = hex::decode(&key_hex).context("decoding encryption_key as hex")?;
            Some(Arc::new(
                CredentialEncryptor::new(&key_bytes).context("creating credential encryptor")?,
            ))
        };

        let db_manager = DatabaseManager::new(&self.config.database, encryptor.clone())
            .await
            .context("initializing database manager")?;

        db_manager
            .setup()
            .await
            .context("running driver database migrations")?;

        info!("Database initialized");

        let ps = Arc::new(PlayerStore::new(db_manager.stdlib_pool().clone()));
        // Ensure runtime data directories exist.
        let data_path = std::path::Path::new(&self.config.world.data_path);
        std::fs::create_dir_all(data_path)
            .with_context(|| format!("creating data directory: {}", data_path.display()))?;
        let world_path = self.config.world.resolved_path();
        let git_path = self.config.world.resolved_git_path();
        std::fs::create_dir_all(&world_path)
            .with_context(|| format!("creating world directory: {}", world_path.display()))?;
        std::fs::create_dir_all(&git_path)
            .with_context(|| format!("creating git directory: {}", git_path.display()))?;

        let rm = Arc::new(RepoManager::new(git_path));
        let ws = Arc::new(Workspace::new(world_path, Arc::clone(&rm)));

        // Merge request manager
        let mr_store = crate::persistence::merge_request_store::MergeRequestStore::new(
            db_manager.driver_pool().clone(),
        );
        let mrm = Arc::new(MergeRequestManager::new(
            mr_store,
            Arc::clone(&rm),
            Arc::clone(&ws),
            crate::git::merge_request_manager::ReviewPolicy::default(),
        ));

        self.player_store = Some(Arc::clone(&ps));
        self.repo_manager = Some(Arc::clone(&rm));
        self.workspace = Some(Arc::clone(&ws));
        self.merge_request_manager = Some(mrm);
        self.encryptor = encryptor;
        self.db_manager = Some(db_manager);

        info!("PlayerStore, RepoManager, Workspace, and MergeRequestManager initialized");

        self.ensure_system_repos().await?;

        // Send stdlib DB URL to the adapter after the system stdlib repo has
        // been provisioned and checked out so the adapter can switch to it
        // during configure handling.
        let admin_password = self.config.database.admin_password.as_deref().unwrap_or("");
        let stdlib_db_url = format!(
            "postgres://{}:{}@{}:{}/{}",
            self.config.database.admin_user,
            admin_password,
            self.config.database.host,
            self.config.database.port,
            self.config.database.stdlib_db,
        );
        self.send_configure(stdlib_db_url).await?;
        info!("Sent stdlib DB URL to adapter");

        Ok(())
    }

    async fn ensure_system_repos(&self) -> Result<()> {
        let Some(repo_manager) = &self.repo_manager else {
            return Ok(());
        };
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };

        if !repo_manager.repo_exists("system", "stdlib") {
            let stdlib_files = self.bootstrap_stdlib_files()?;
            repo_manager
                .create_repo("system", "stdlib", true, Some(&stdlib_files))
                .context("bootstrapping system/stdlib repo")?;
        }
        workspace
            .checkout("system", "stdlib")
            .context("checking out system/stdlib")?;

        let template_names = self.bootstrap_template_names();
        for template_name in template_names {
            let repo_name = template_repo_name(&template_name);
            if !repo_manager.repo_exists("system", &repo_name) {
                match self.bootstrap_template_files(&template_name).await {
                    Ok(files) => {
                        if let Err(e) =
                            repo_manager.create_repo("system", &repo_name, true, Some(&files))
                        {
                            warn!(
                                template = %template_name,
                                repo = %repo_name,
                                error = %e,
                                "failed to bootstrap system template repo"
                            );
                            continue;
                        }
                    }
                    Err(e) => {
                        warn!(
                            template = %template_name,
                            repo = %repo_name,
                            error = %e,
                            "failed to load bootstrap template files"
                        );
                        continue;
                    }
                }
            }

            if let Err(e) = workspace.checkout("system", &repo_name) {
                warn!(
                    template = %template_name,
                    repo = %repo_name,
                    error = %e,
                    "failed to checkout system template repo"
                );
            }
        }

        Ok(())
    }

    fn system_templates_exist(&self) -> bool {
        let system_dir = self.config.world.resolved_path().join("system");
        if !system_dir.is_dir() {
            return false;
        }
        match std::fs::read_dir(system_dir) {
            Ok(entries) => entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.starts_with("template_"))
                    .unwrap_or(false)
            }),
            Err(_) => false,
        }
    }

    async fn refresh_template_registry(&self) -> Result<()> {
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };

        let system_path = workspace.world_path().join("system");
        let mut discovered = HashMap::new();
        if system_path.is_dir() {
            for entry in std::fs::read_dir(&system_path)
                .with_context(|| format!("reading {}", system_path.display()))?
            {
                let entry = entry?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(repo_name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !repo_name.starts_with("template_") {
                    continue;
                }
                let metadata_path = path.join("template.yml");
                if !metadata_path.is_file() {
                    continue;
                }
                let contents = std::fs::read_to_string(&metadata_path)
                    .with_context(|| format!("reading {}", metadata_path.display()))?;
                let metadata: TemplateMetadata = serde_yaml::from_str(&contents)
                    .with_context(|| format!("parsing {}", metadata_path.display()))?;
                discovered.insert(
                    metadata.name.clone(),
                    RegisteredTemplate {
                        name: metadata.name.clone(),
                        repo_namespace: "system".into(),
                        repo_name: repo_name.to_string(),
                        path: path.clone(),
                        metadata,
                    },
                );
            }
        }

        *self.template_registry.write().await = discovered;
        Ok(())
    }

    fn bootstrap_template_names(&self) -> Vec<String> {
        if !self.config.bootstrap.area_templates.is_empty() {
            return self.config.bootstrap.area_templates.clone();
        }

        vec![
            "default".into(),
            "lpc".into(),
            "rust".into(),
            "kotlin:ktor".into(),
            "kotlin:quarkus".into(),
            "kotlin:spring-boot".into(),
        ]
    }

    fn bootstrap_stdlib_files(&self) -> Result<HashMap<String, String>> {
        match self.config.bootstrap.stdlib_template.as_str() {
            "ruby" => Ok(with_stdlib_metadata_file(collect_template_files(Path::new(
                "bootstrap/ruby/stdlib",
            ))?)),
            other => Err(anyhow::anyhow!(
                "unsupported bootstrap stdlib template: {}",
                other
            )),
        }
    }

    async fn bootstrap_template_files(&self, template_name: &str) -> Result<HashMap<String, String>> {
        if let Some(files) = self.area_templates.read().await.get(template_name).cloned() {
            return Ok(with_template_metadata_file(template_name, files));
        }

        let disk_files = scan_disk_template_by_name(template_name)?;
        Ok(with_template_metadata_file(template_name, disk_files))
    }

    /// Return a reference to the PlayerStore, if initialized.
    pub fn player_store(&self) -> Option<&Arc<PlayerStore>> {
        self.player_store.as_ref()
    }

    /// Return a reference to the RepoManager, if initialized.
    pub fn repo_manager(&self) -> Option<&Arc<RepoManager>> {
        self.repo_manager.as_ref()
    }

    /// Return a reference to the Workspace, if initialized.
    pub fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.workspace.as_ref()
    }

    /// Shut down the adapter manager and clean up resources.
    pub fn shutdown(&mut self) {
        self.adapter_manager.shutdown();
    }

    /// Unified event loop: handle SSH commands, adapter messages, and MOP RPC
    /// requests from web handlers using `tokio::select!`.
    async fn run_event_loop(&mut self, mut ssh_cmd_rx: mpsc::Receiver<SshCommand>) -> Result<()> {
        // Take the receiver out of the Option so we can use it in the loop.
        // If it was already taken (shouldn't happen), create a dummy channel.
        let mut mop_rpc_rx = self
            .mop_rpc_rx
            .take()
            .unwrap_or_else(|| mpsc::channel::<MopRequest>(1).1);
        let mut server_command_rx = self
            .server_command_rx
            .take()
            .unwrap_or_else(|| mpsc::channel::<ServerCommand>(1).1);

        loop {
            tokio::select! {
                Some(cmd) = ssh_cmd_rx.recv() => {
                    self.handle_ssh_command(cmd).await;
                }
                Some(sourced) = self.adapter_manager.recv() => {
                    self.handle_adapter_message(sourced).await;
                }
                Some(rpc_req) = mop_rpc_rx.recv() => {
                    self.handle_mop_rpc_request(rpc_req).await;
                }
                Some(cmd) = server_command_rx.recv() => {
                    self.handle_server_command(cmd).await;
                }
                else => {
                    info!("Event loop: all channels closed, shutting down");
                    break;
                }
            }
        }
        self.adapter_manager.shutdown();
        Ok(())
    }

    async fn handle_server_command(&self, cmd: ServerCommand) {
        match cmd {
            ServerCommand::RepoUpdated { namespace, name } => {
                if let Err(e) = self.handle_repo_updated(&namespace, &name, None).await {
                    warn!(namespace = %namespace, name = %name, error = %e, "repo update handling failed");
                }
            }
        }
    }

    /// Handle a MOP RPC request submitted by a web handler.
    ///
    /// Assigns a unique request ID, stores the response channel, and sends
    /// the message to the adapter. The adapter's `CallResult`/`CallError`
    /// response will be routed back via [`complete_rpc`].
    async fn handle_mop_rpc_request(&mut self, rpc_req: MopRequest) {
        let rpc_id = self.next_rpc_id;
        self.next_rpc_id += 1;
        let MopRequest {
            message,
            target_language,
            response_tx,
        } = rpc_req;

        // Replace the request_id in the message with our assigned ID.
        let message = replace_request_id(message, rpc_id);

        // Store the oneshot sender so we can deliver the response later.
        self.pending_rpc.insert(rpc_id, response_tx);

        // Area-specific web RPCs can target a non-primary adapter.
        let target_language =
            target_language.unwrap_or_else(|| self.primary_language().to_string());
        if let Err(e) = self
            .adapter_manager
            .send_to(&target_language, message)
            .await
        {
            error!(%e, rpc_id, "failed to send MOP RPC request to adapter");
            if let Some(tx) = self.pending_rpc.remove(&rpc_id) {
                let _ = tx.send(Err(format!("adapter send failed: {e}")));
            }
        }
    }

    /// Complete a pending MOP RPC call by delivering the adapter's response
    /// to the waiting web handler.
    ///
    /// Returns `true` if a pending RPC was found and completed.
    fn complete_rpc(&mut self, request_id: u64, result: Result<Value, String>) -> bool {
        if let Some(tx) = self.pending_rpc.remove(&request_id) {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }

    /// Handle a command arriving from an SSH connection.
    async fn handle_ssh_command(&mut self, cmd: SshCommand) {
        match cmd {
            SshCommand::NewSession {
                username,
                output_tx,
                session_id_tx,
                ..
            } => {
                let (session_id, mut output_rx) = self.create_session();

                // Send the assigned session ID back to the SSH handler.
                let _ = session_id_tx.send(session_id);

                info!(session_id, %username, "SSH session created");

                // Bridge: forward session output (String) to SSH channel (Vec<u8>).
                let ssh_output = output_tx;
                tokio::spawn(async move {
                    while let Some(text) = output_rx.recv().await {
                        if ssh_output.send(text.into_bytes()).await.is_err() {
                            break;
                        }
                    }
                });

                // Notify the adapter that a player session has started.
                if let Err(e) = self.session_start(session_id, username).await {
                    error!(session_id, %e, "failed to notify adapter of session start");
                }
            }
            SshCommand::Input { session_id, line } => {
                if let Err(e) = self.session_input(session_id, line).await {
                    error!(session_id, %e, "failed to forward session input");
                }
            }
            SshCommand::Disconnect { session_id } => {
                info!(session_id, "SSH session disconnected");
                if let Err(e) = self.session_end(session_id).await {
                    error!(session_id, %e, "failed to end session");
                }
            }
        }
    }

    /// Handle a message arriving from a language adapter.
    pub async fn handle_adapter_message(&mut self, sourced: SourcedMessage) {
        let source_lang = sourced.language;
        match sourced.message {
            AdapterMessage::SessionOutput { session_id, text } => {
                self.sessions.send_output(session_id, text);
            }
            AdapterMessage::AreaLoaded { area_id } => {
                info!(%area_id, "Area loaded successfully");
                self.loaded_areas.write().await.insert(area_id.to_string());
            }
            AdapterMessage::AreaError { area_id, error } => {
                error!(%area_id, %error, "Area failed to load");
            }
            AdapterMessage::Log {
                level,
                message,
                area,
            } => {
                match level.as_str() {
                    "error" => error!(area = area.as_deref(), "{message}"),
                    "warn" => warn!(area = area.as_deref(), "{message}"),
                    _ => info!(area = area.as_deref(), "{message}"),
                }
                // Append to master log file
                if let Some(ref log_path) = self.master_log_path {
                    Self::append_master_log(log_path, &level, &message, area.as_deref());
                }
            }
            AdapterMessage::CallResult {
                request_id,
                result,
                cache: _,
            } => {
                if !self.complete_rpc(request_id, Ok(result)) {
                    warn!(request_id, "received CallResult for unknown RPC request");
                }
            }
            AdapterMessage::CallError { request_id, error } => {
                if !self.complete_rpc(request_id, Err(error.clone())) {
                    warn!(request_id, %error, "received CallError for unknown RPC request");
                }
            }
            AdapterMessage::DriverRequest {
                request_id,
                action,
                params,
            } => {
                info!(%request_id, %action, source = %source_lang, "Processing driver request");
                let response = self
                    .handle_driver_request(request_id, &action, params)
                    .await;
                if let Err(e) = self.adapter_manager.send_to(&source_lang, response).await {
                    error!(%e, "failed to send request response to {}", source_lang);
                }
            }
            other => {
                warn!(?other, "unhandled adapter message");
            }
        }
    }

    // -----------------------------------------------------------------
    // Driver request dispatch
    // -----------------------------------------------------------------

    /// Dispatch a driver request to the appropriate handler.
    async fn handle_driver_request(
        &mut self,
        request_id: u64,
        action: &str,
        params: Value,
    ) -> DriverMessage {
        // Mutable handlers (need &mut self).
        if action == "set_area_template" {
            return self.handle_set_area_template(request_id, params).await;
        }
        if action == "register_area_web" {
            return self.handle_register_area_web(request_id, params).await;
        }

        // Handlers that don't require PlayerStore.
        match action {
            "repo_create" => return self.handle_repo_create(request_id, params).await,
            "repo_list" => return self.handle_repo_list(request_id, params).await,
            "repo_grant" => return self.handle_repo_grant(request_id, params).await,
            "repo_revoke" => return self.handle_repo_revoke(request_id, params).await,
            "repo_check_access" => return self.handle_repo_check_access(request_id, params).await,
            "area_reload" => return self.handle_area_reload(request_id, params).await,
            "workspace_diff" => return self.handle_workspace_diff(request_id, params).await,
            "workspace_log" => return self.handle_workspace_log(request_id, params).await,
            "workspace_commit" => return self.handle_workspace_commit(request_id, params).await,
            "workspace_pull" => return self.handle_workspace_pull(request_id, params).await,
            "workspace_checkout" => {
                return self.handle_workspace_checkout(request_id, params).await
            }
            "workspace_checkout_branch" => {
                return self
                    .handle_workspace_checkout_branch(request_id, params)
                    .await
            }
            "workspace_branches" => {
                return self.handle_workspace_branches(request_id, params).await
            }
            "workspace_create_branch" => {
                return self
                    .handle_workspace_create_branch(request_id, params)
                    .await
            }
            "mr_create" => return self.handle_mr_create(request_id, params).await,
            "mr_get" => return self.handle_mr_get(request_id, params).await,
            "mr_list_all" => return self.handle_mr_list_all(request_id, params).await,
            "mr_approve" => return self.handle_mr_approve(request_id, params).await,
            "mr_reject" => return self.handle_mr_reject(request_id, params).await,
            "mr_merge" => return self.handle_mr_merge(request_id, params).await,
            "mr_close" => return self.handle_mr_close(request_id, params).await,
            _ => {}
        }

        // Handlers that require PlayerStore.
        let ps = match &self.player_store {
            Some(ps) => ps,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "database not configured".into(),
                };
            }
        };

        match action {
            "player_find" => self.handle_player_find(request_id, ps, params).await,
            "player_create" => self.handle_player_create(request_id, ps, params).await,
            "player_authenticate" => {
                self.handle_player_authenticate(request_id, ps, params)
                    .await
            }
            "session_create" => self.handle_session_create(request_id, ps, params).await,
            "session_validate" => self.handle_session_validate(request_id, ps, params).await,
            "session_destroy" => self.handle_session_destroy(request_id, ps, params).await,
            "player_switch_character" => {
                self.handle_player_switch_character(request_id, ps, params)
                    .await
            }
            "player_add_character" => {
                self.handle_player_add_character(request_id, ps, params)
                    .await
            }
            "set_role" => self.handle_set_role(request_id, ps, params).await,
            _ => DriverMessage::RequestError {
                request_id,
                error: format!("unknown action: '{}'", action),
            },
        }
    }

    /// Find a player by username. Returns player data or null.
    async fn handle_player_find(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let username = match get_string_param(&params, "username") {
            Some(u) => u,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'username' parameter".into(),
                };
            }
        };

        match ps.find(&username).await {
            Ok(Some(player)) => {
                // Also fetch characters for this player.
                let characters = ps.list_characters(&username).await.unwrap_or_default();
                let char_list: Vec<Value> = characters
                    .into_iter()
                    .map(|c| {
                        let mut m = HashMap::new();
                        m.insert("name".into(), Value::String(c.name));
                        m.insert("id".into(), Value::Int(c.id as i64));
                        Value::Map(m)
                    })
                    .collect();

                let mut result = HashMap::new();
                result.insert("id".into(), Value::String(player.id));
                result.insert("role".into(), Value::String(player.role));
                result.insert(
                    "active_character".into(),
                    player
                        .active_character
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
                result.insert("characters".into(), Value::Array(char_list));

                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::Map(result),
                }
            }
            Ok(None) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Null,
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("player_find failed: {}", e),
            },
        }
    }

    /// Create a new player account with an initial character.
    async fn handle_player_create(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let username = match get_string_param(&params, "username") {
            Some(u) => u,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'username' parameter".into(),
                };
            }
        };
        let password = match get_string_param(&params, "password") {
            Some(p) => p,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'password' parameter".into(),
                };
            }
        };
        let character = get_string_param(&params, "character").unwrap_or_default();

        // Hash the password.
        let password_hash = match PlayerStore::hash_password(&password) {
            Ok(h) => h,
            Err(e) => {
                return DriverMessage::RequestError {
                    request_id,
                    error: format!("failed to hash password: {}", e),
                };
            }
        };

        // Create the account.
        if let Err(e) = ps.create(&username, &password_hash).await {
            return DriverMessage::RequestError {
                request_id,
                error: format!("failed to create player: {}", e),
            };
        }

        // Create the initial character and set it as active.
        if !character.is_empty() {
            if let Err(e) = ps.add_character(&username, &character).await {
                warn!(%e, "failed to add initial character");
            }
            if let Err(e) = ps.switch_character(&username, &character).await {
                warn!(%e, "failed to set active character");
            }
        }

        // Create a default git area for the new builder account.
        if let Some(rm) = &self.repo_manager {
            let template_name = self.config.adapters.default_template.as_deref().unwrap_or("default");
            let template_registry = self.template_registry.read().await;
            let create_result = if let Some(template) = template_registry
                .get(template_name)
                .or_else(|| template_registry.get("default"))
                .or_else(|| template_registry.values().next())
            {
                rm.create_repo_from_template_repo(&username, &username, &template.path, "main")
            } else {
                let templates = self.area_templates.read().await;
                let template = self
                    .config
                    .adapters
                    .default_template
                    .as_ref()
                    .and_then(|name| templates.get(name))
                    .or_else(|| templates.get("default"))
                    .or_else(|| templates.values().next());
                rm.create_repo(&username, &username, true, template)
            };
            drop(template_registry);
            if let Err(e) = create_result {
                warn!(%e, "failed to create default area for new account");
            } else if let Some(ws) = &self.workspace {
                if let Err(e) = ws.checkout(&username, &username) {
                    warn!(%e, "failed to checkout default area for new account");
                }
            }
        }

        DriverMessage::RequestResponse {
            request_id,
            result: Value::Bool(true),
        }
    }

    /// Authenticate a player by username and password.
    async fn handle_player_authenticate(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let username = match get_string_param(&params, "username") {
            Some(u) => u,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'username' parameter".into(),
                };
            }
        };
        let password = match get_string_param(&params, "password") {
            Some(p) => p,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'password' parameter".into(),
                };
            }
        };

        match ps.authenticate(&username, &password).await {
            Ok(AuthResult::Success(player)) => {
                let mut data = HashMap::new();
                data.insert("role".into(), Value::String(player.role));
                data.insert(
                    "active_character".into(),
                    player
                        .active_character
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );

                let mut result = HashMap::new();
                result.insert("success".into(), Value::Bool(true));
                result.insert("data".into(), Value::Map(data));

                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::Map(result),
                }
            }
            Ok(AuthResult::WrongPassword | AuthResult::NotFound) => {
                let mut result = HashMap::new();
                result.insert("success".into(), Value::Bool(false));

                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::Map(result),
                }
            }
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("authentication failed: {}", e),
            },
        }
    }

    /// Create a session token for the given account.
    async fn handle_session_create(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let account = match get_string_param(&params, "account") {
            Some(a) => a,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'account' parameter".into(),
                };
            }
        };

        match ps.create_session(&account).await {
            Ok(token) => DriverMessage::RequestResponse {
                request_id,
                result: Value::String(token),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to create session: {}", e),
            },
        }
    }

    /// Destroy a session by token.
    async fn handle_session_destroy(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let token = match get_string_param(&params, "token") {
            Some(t) => t,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'token' parameter".into(),
                };
            }
        };

        match ps.destroy_session(&token).await {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to destroy session: {}", e),
            },
        }
    }

    /// Validate a session token for the given account.
    async fn handle_session_validate(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let account = match get_string_param(&params, "account") {
            Some(a) => a,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'account' parameter".into(),
                };
            }
        };
        let token = match get_string_param(&params, "token") {
            Some(t) => t,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'token' parameter".into(),
                };
            }
        };

        match ps.valid_session(&account, &token).await {
            Ok(valid) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(valid),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to validate session: {}", e),
            },
        }
    }

    /// Switch the active character for a player.
    async fn handle_player_switch_character(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let account = match get_string_param(&params, "account") {
            Some(a) => a,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'account' parameter".into(),
                };
            }
        };
        let character = match get_string_param(&params, "character") {
            Some(c) => c,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'character' parameter".into(),
                };
            }
        };

        match ps.switch_character(&account, &character).await {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to switch character: {}", e),
            },
        }
    }

    /// Add a new character to a player account.
    async fn handle_player_add_character(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let account = match get_string_param(&params, "account") {
            Some(a) => a,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'account' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(n) => n,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };

        match ps.add_character(&account, &name).await {
            Ok(_id) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to add character: {}", e),
            },
        }
    }

    /// Set the role for a player account.
    async fn handle_set_role(
        &self,
        request_id: u64,
        ps: &PlayerStore,
        params: Value,
    ) -> DriverMessage {
        let username = match get_string_param(&params, "username") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'username' parameter".into(),
                };
            }
        };
        let role = match get_string_param(&params, "role") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'role' parameter".into(),
                };
            }
        };

        match ps.set_role(&username, &role).await {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to set role: {}", e),
            },
        }
    }

    /// Store the area template files provided by the adapter.
    /// The adapter sends a map of `{ "files": { "path": "content", ... } }`.
    async fn handle_set_area_template(&self, request_id: u64, params: Value) -> DriverMessage {
        if !self.template_registry.read().await.is_empty() || self.system_templates_exist() {
            return DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            };
        }

        let (name, files) = match &params {
            Value::Map(m) => {
                let name = match m.get("name") {
                    Some(Value::String(s)) => s.clone(),
                    _ => "default".to_string(),
                };
                match m.get("files") {
                    Some(Value::Map(files_map)) => {
                        let mut template = HashMap::new();
                        for (path, content) in files_map {
                            if let Value::String(content_str) = content {
                                template.insert(path.clone(), content_str.clone());
                            }
                        }
                        (name, template)
                    }
                    _ => {
                        return DriverMessage::RequestError {
                            request_id,
                            error: "missing 'files' map parameter".into(),
                        };
                    }
                }
            }
            _ => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "params must be a map".into(),
                };
            }
        };

        let count = files.len();
        info!(name = %name, count, "Area template set");
        self.area_templates.write().await.insert(name, files);

        DriverMessage::RequestResponse {
            request_id,
            result: Value::Bool(true),
        }
    }

    /// Scan adapter template directories on disk and register any templates
    /// that weren't already provided by a running adapter.  This ensures
    /// templates are available in the UI even when an adapter is disabled.
    async fn scan_disk_templates(&self) {
        // Ruby default template
        let ruby_default = std::path::Path::new("bootstrap/ruby/stdlib/templates/area");
        if ruby_default.is_dir() {
            let existing = self.area_templates.read().await;
            let needs_insert = !existing.contains_key("default");
            drop(existing);
            if needs_insert {
                match collect_template_files(ruby_default) {
                    Ok(files) => {
                        let count = files.len();
                        info!(name = "default", count, "Disk-scanned area template registered");
                        self.area_templates
                            .write()
                            .await
                            .insert("default".into(), files);
                    }
                    Err(e) => warn!(error = %e, "Failed to read Ruby default template"),
                }
            }
        }

        // JVM templates: base + overlays in bootstrap/jvm/templates/area/
        let jvm_base = std::path::Path::new("bootstrap/jvm/templates/area/base");
        let jvm_overlays = std::path::Path::new("bootstrap/jvm/templates/area/overlays");

        if jvm_base.is_dir() && jvm_overlays.is_dir() {
            let base_files = match collect_template_files(jvm_base) {
                Ok(f) => f,
                Err(e) => {
                    warn!(error = %e, "Failed to read JVM base template");
                    return;
                }
            };

            if let Ok(entries) = std::fs::read_dir(jvm_overlays) {
                let existing = self.area_templates.read().await;
                let mut to_insert = Vec::new();

                for entry in entries.flatten() {
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let overlay_name = entry.file_name().to_string_lossy().to_string();
                    let template_name = format!("kotlin:{overlay_name}");

                    // Skip if adapter already registered this template
                    if existing.contains_key(&template_name) {
                        continue;
                    }

                    match collect_template_files(&entry.path()) {
                        Ok(overlay_files) => {
                            // Merge: base files + overlay files (overlay wins)
                            let mut merged = base_files.clone();
                            merged.extend(overlay_files);
                            to_insert.push((template_name, merged));
                        }
                        Err(e) => {
                            warn!(overlay = %overlay_name, error = %e, "Failed to read JVM overlay");
                        }
                    }
                }
                drop(existing);

                if !to_insert.is_empty() {
                    let mut templates = self.area_templates.write().await;
                    for (name, files) in to_insert {
                        let count = files.len();
                        info!(name = %name, count, "Disk-scanned area template registered");
                        templates.insert(name, files);
                    }
                }
            }
        }

        // LPC/Rust templates: each subdirectory is a self-contained template
        let lpc_templates = std::path::Path::new("bootstrap/lpc/templates/area");

        if lpc_templates.is_dir() {
            if let Ok(entries) = std::fs::read_dir(lpc_templates) {
                let existing = self.area_templates.read().await;
                let mut to_insert = Vec::new();

                for entry in entries.flatten() {
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let template_name = entry.file_name().to_string_lossy().to_string();

                    // Skip if adapter already registered this template
                    if existing.contains_key(&template_name) {
                        continue;
                    }

                    match collect_template_files(&entry.path()) {
                        Ok(files) => {
                            to_insert.push((template_name, files));
                        }
                        Err(e) => {
                            warn!(
                                template = %entry.file_name().to_string_lossy(),
                                error = %e,
                                "Failed to read LPC/Rust template"
                            );
                        }
                    }
                }
                drop(existing);

                if !to_insert.is_empty() {
                    let mut templates = self.area_templates.write().await;
                    for (name, files) in to_insert {
                        let count = files.len();
                        info!(name = %name, count, "Disk-scanned area template registered");
                        templates.insert(name, files);
                    }
                }
            }
        }
    }

    /// Register a per-area web socket path for API proxying.
    async fn handle_register_area_web(&self, request_id: u64, params: Value) -> DriverMessage {
        let area_key = get_string_param(&params, "area_key").unwrap_or_default();
        let socket_path = get_string_param(&params, "socket_path").unwrap_or_default();
        info!(area_key = %area_key, socket_path = %socket_path, "Area web socket registered");
        self.area_web_sockets
            .write()
            .await
            .insert(area_key, socket_path);
        DriverMessage::RequestResponse {
            request_id,
            result: Value::Bool(true),
        }
    }

    /// Create a new git repository.
    async fn handle_repo_create(&self, request_id: u64, params: Value) -> DriverMessage {
        let rm = match &self.repo_manager {
            Some(rm) => rm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "repo_manager not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let seed = match &params {
            Value::Map(m) => m
                .get("seed")
                .and_then(|v| match v {
                    Value::Bool(b) => Some(*b),
                    _ => None,
                })
                .unwrap_or(true),
            _ => true,
        };

        let template_name = match &params {
            Value::Map(m) => m
                .get("template")
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .or_else(|| self.config.adapters.default_template.clone()),
            _ => self.config.adapters.default_template.clone(),
        };

        let system_templates = self.template_registry.read().await;
        let result = if !system_templates.is_empty() {
            let template = template_name
                .as_ref()
                .and_then(|name| system_templates.get(name))
                .or_else(|| system_templates.get("default"))
                .or_else(|| system_templates.values().next());

            match template {
                Some(template) => {
                    rm.create_repo_from_template_repo(&ns, &name, &template.path, "main")
                }
                None => Err(anyhow::anyhow!("no templates available")),
            }
        } else {
            let templates = self.area_templates.read().await;
            let template = template_name
                .as_ref()
                .and_then(|name| templates.get(name))
                .or_else(|| templates.get("default"))
                .or_else(|| templates.values().next());

            rm.create_repo(&ns, &name, seed, template)
        };

        match result {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to create repo: {}", e),
            },
        }
    }

    /// List repositories in a namespace.
    async fn handle_repo_list(&self, request_id: u64, params: Value) -> DriverMessage {
        let rm = match &self.repo_manager {
            Some(rm) => rm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "repo_manager not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace")
            .or_else(|| get_string_param(&params, "owner"))
        {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' (or 'owner') parameter".into(),
                };
            }
        };

        match rm.list_repos(&ns) {
            Ok(repos) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Array(repos.into_iter().map(Value::String).collect()),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to list repos: {}", e),
            },
        }
    }

    /// Check whether a user has access to a repository.
    async fn handle_repo_check_access(&self, request_id: u64, params: Value) -> DriverMessage {
        let rm = match &self.repo_manager {
            Some(rm) => rm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "repo_manager not configured".into(),
                };
            }
        };
        let username = match get_string_param(&params, "username") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'username' parameter".into(),
                };
            }
        };
        let namespace = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let level_str = get_string_param(&params, "level").unwrap_or_else(|| "read_only".into());
        let level = match level_str.as_str() {
            "read_write" => crate::git::AccessLevel::ReadWrite,
            _ => crate::git::AccessLevel::ReadOnly,
        };

        let allowed = rm.can_access(&username, &namespace, &name, &level);
        DriverMessage::RequestResponse {
            request_id,
            result: Value::Bool(allowed),
        }
    }

    async fn handle_repo_grant(&self, request_id: u64, params: Value) -> DriverMessage {
        let rm = match &self.repo_manager {
            Some(rm) => rm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "repo_manager not configured".into(),
                };
            }
        };

        let namespace = match get_string_param(&params, "namespace")
            .or_else(|| get_string_param(&params, "owner"))
        {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' (or 'owner') parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let target_user = match get_string_param(&params, "target_user") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'target_user' parameter".into(),
                };
            }
        };
        let level = match get_string_param(&params, "level").as_deref() {
            Some("read_write") => crate::git::AccessLevel::ReadWrite,
            _ => crate::git::AccessLevel::ReadOnly,
        };

        match rm.grant_access(&namespace, &name, &target_user, level) {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to grant repo access: {}", e),
            },
        }
    }

    async fn handle_repo_revoke(&self, request_id: u64, params: Value) -> DriverMessage {
        let rm = match &self.repo_manager {
            Some(rm) => rm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "repo_manager not configured".into(),
                };
            }
        };

        let namespace = match get_string_param(&params, "namespace")
            .or_else(|| get_string_param(&params, "owner"))
        {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' (or 'owner') parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let target_user = match get_string_param(&params, "target_user") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'target_user' parameter".into(),
                };
            }
        };

        match rm.revoke_access(&namespace, &name, &target_user) {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to revoke repo access: {}", e),
            },
        }
    }

    /// Reload an area by sending ReloadArea to the appropriate adapter.
    async fn handle_area_reload(&self, request_id: u64, params: Value) -> DriverMessage {
        if self.adapter_languages.is_empty() {
            return DriverMessage::RequestError {
                request_id,
                error: "no adapter connected".into(),
            };
        }
        let area_id = match get_string_param(&params, "area_id") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'area_id' parameter".into(),
                };
            }
        };
        let path = match get_string_param(&params, "path") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'path' parameter".into(),
                };
            }
        };
        let language = self.language_for_area(&path);

        let db_url = if let Some(ref db_mgr) = self.db_manager {
            let parts: Vec<&str> = area_id.splitn(2, '/').collect();
            let (ns, name) = if parts.len() == 2 {
                (parts[0], parts[1])
            } else {
                ("", area_id.as_str())
            };
            // Provision if not yet provisioned (idempotent)
            if let Err(e) = db_mgr.provision_area_db(ns, name).await {
                warn!(area_id, %e, "Failed to provision area database on reload");
            }
            db_mgr.get_area_db_url(ns, name).await.unwrap_or(None)
        } else {
            None
        };

        match self
            .adapter_manager
            .send_to(
                &language,
                DriverMessage::ReloadArea {
                    area_id: {
                        let parts: Vec<&str> = area_id.splitn(2, '/').collect();
                        if parts.len() == 2 {
                            AreaId::new(parts[0], parts[1])
                        } else {
                            AreaId::new("", &area_id)
                        }
                    },
                    path,
                    db_url,
                },
            )
            .await
        {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("failed to send reload: {}", e),
            },
        }
    }

    async fn handle_repo_updated(
        &self,
        namespace: &str,
        name: &str,
        committed_branch: Option<(&str, &[crate::git::workspace::DiffEntry])>,
    ) -> Result<()> {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => return Ok(()),
        };

        if namespace == "system" && name == "stdlib" {
            let changed = if let Some((branch, entries)) = committed_branch {
                if branch != "main" {
                    return Ok(());
                }
                entries.iter().map(|entry| entry.path.clone()).collect::<Vec<_>>()
            } else {
                ws.pull_with_changed_files(namespace, name, "main")?
                    .into_iter()
                    .map(|entry| entry.path)
                    .collect::<Vec<_>>()
            };

            if changed.is_empty() {
                return Ok(());
            }

            let subsystem = classify_stdlib_subsystem(&changed);
            self.adapter_manager.send_to(
                self.primary_language(),
                DriverMessage::ReloadStdlib {
                    subsystem: subsystem.to_string(),
                },
            ).await?;

            self.propagate_stdlib_reload(&changed).await?;
            return Ok(());
        }

        if let Some((branch, entries)) = committed_branch {
            self.handle_area_branch_update(namespace, name, branch, entries).await?;
            return Ok(());
        }

        let main_changes = ws.pull_with_changed_files(namespace, name, "main")?;
        self.handle_area_branch_update(namespace, name, "main", &main_changes)
            .await?;

        let develop_changes = ws.pull_with_changed_files(namespace, name, "develop")?;
        self.handle_area_branch_update(namespace, name, "develop", &develop_changes)
            .await?;

        Ok(())
    }

    async fn handle_area_branch_update(
        &self,
        namespace: &str,
        name: &str,
        branch: &str,
        changes: &[crate::git::workspace::DiffEntry],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }

        let ws = match &self.workspace {
            Some(ws) => ws,
            None => return Ok(()),
        };

        let area_path = if branch == "develop" {
            ws.dev_path(namespace, name)
        } else {
            ws.workspace_path(namespace, name)
        };

        self.trigger_spa_build(namespace, name, branch, &area_path);

        let area_path_str = area_path.to_string_lossy().to_string();
        if !std::path::Path::new(&area_path_str).join(".meta.yml").exists() {
            return Ok(());
        }

        let area_id = AreaId::new(namespace, name);
        self.send_area_reload(area_id, area_path_str).await
    }

    fn trigger_spa_build(&self, namespace: &str, name: &str, branch: &str, area_path: &Path) {
        let Some(build_manager) = &self.build_manager else {
            return;
        };

        let (area_key, base_url) = if branch == "develop" {
            (
                format!("{namespace}/{name}@dev"),
                format!("/project/{namespace}/{name}@dev/"),
            )
        } else {
            (
                format!("{namespace}/{name}"),
                format!("/project/{namespace}/{name}/"),
            )
        };

        if BuildManager::is_spa(area_path) {
            build_manager.trigger_build(area_key, area_path.to_path_buf(), base_url);
        }
    }

    async fn send_area_reload(&self, area_id: AreaId, path: String) -> Result<()> {
        let language = self.language_for_area(&path);
        self.ensure_runtime_dependency_roots().await;
        self.register_area_runtime_dependencies(&area_id, &path, &language)
            .await;
        let db_url = if let Some(ref db_mgr) = self.db_manager {
            if let Err(e) = db_mgr
                .provision_area_db(&area_id.namespace, &area_id.name)
                .await
            {
                warn!(area = %area_id, error = %e, "failed to provision area database on reload");
            }
            db_mgr
                .get_area_db_url(&area_id.namespace, &area_id.name)
                .await
                .unwrap_or(None)
        } else {
            None
        };

        self.adapter_manager
            .send_to(
                &language,
                DriverMessage::ReloadArea {
                    area_id,
                    path,
                    db_url,
                },
            )
            .await
    }

    async fn ensure_runtime_dependency_roots(&self) {
        let mut tree = self.version_tree.write().await;
        for (program, deps) in ruby_stdlib_dependency_roots() {
            if tree.get(program).is_none() {
                tree.register(program, "ruby", deps);
            }
        }
    }

    async fn register_area_runtime_dependencies(&self, area_id: &AreaId, path: &str, language: &str) {
        if area_id.namespace == "system" && area_id.name == "stdlib" {
            return;
        }

        let deps = area_dependency_paths(path, language);
        let program_path = area_program_path(area_id);
        self.version_tree
            .write()
            .await
            .register(&program_path, language, deps);
    }

    async fn propagate_stdlib_reload(&self, changed_paths: &[String]) -> Result<()> {
        self.ensure_runtime_dependency_roots().await;

        let affected_programs = {
            let mut tree = self.version_tree.write().await;
            let roots = affected_stdlib_roots(changed_paths);
            let mut affected = std::collections::BTreeSet::new();
            for root in &roots {
                let _ = tree.bump_version(root);
                affected.insert((*root).to_string());
                for dependent in tree.walk_dependents(root) {
                    affected.insert(dependent);
                }
            }
            affected
        };

        let mut reloaded_area_ids = std::collections::BTreeSet::new();
        let mut invalidated_object_ids = Vec::new();
        {
            let mut state_store = self.state_store.write().await;
            for program in &affected_programs {
                if !program.starts_with("area:") {
                    continue;
                }
                invalidated_object_ids.extend(state_store.objects_by_program(program));
                for object_id in state_store.objects_by_program(program) {
                    if let Some(version) = self
                        .version_tree
                        .read()
                        .await
                        .version_of(program)
                    {
                        state_store.upgrade_program(object_id, version);
                    }
                }
                if let Some(area_id) = parse_area_program_path(program) {
                    reloaded_area_ids.insert(area_id.to_string());
                }
            }
        }

        if !invalidated_object_ids.is_empty() {
            self.object_broker
                .write()
                .await
                .invalidate(&invalidated_object_ids);
        }

        let area_lookup: std::collections::HashMap<_, _> = discover_areas(
            &self.config.world.resolved_path().to_string_lossy(),
        )?
        .into_iter()
        .map(|(area_id, path)| (area_id.to_string(), path))
        .collect();

        for area_id in reloaded_area_ids {
            if let Some(path) = area_lookup.get(&area_id) {
                if let Some(parsed) = parse_area_program_path(&format!("area:{area_id}")) {
                    self.send_area_reload(parsed, path.clone()).await?;
                }
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------
    // Workspace handlers
    // -----------------------------------------------------------------

    async fn handle_workspace_diff(&self, request_id: u64, params: Value) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let branch = get_string_param(&params, "branch").unwrap_or_else(|| "develop".into());

        match ws.diff(&ns, &name, &branch) {
            Ok(entries) => {
                let arr: Vec<Value> = entries
                    .into_iter()
                    .map(|e| {
                        let mut m = HashMap::new();
                        m.insert("path".to_string(), Value::String(e.path));
                        m.insert("status".to_string(), Value::String(e.status));
                        Value::Map(m)
                    })
                    .collect();
                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::Array(arr),
                }
            }
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_diff failed: {}", e),
            },
        }
    }

    async fn handle_workspace_log(&self, request_id: u64, params: Value) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let branch = get_string_param(&params, "branch").unwrap_or_else(|| "develop".into());
        let limit = get_int_param(&params, "limit").unwrap_or(20) as usize;

        match ws.log(&ns, &name, &branch, limit) {
            Ok(commits) => {
                let arr: Vec<Value> = commits
                    .into_iter()
                    .map(|c| {
                        let mut m = HashMap::new();
                        m.insert("oid".to_string(), Value::String(c.oid));
                        m.insert("message".to_string(), Value::String(c.message));
                        m.insert("author".to_string(), Value::String(c.author));
                        m.insert("time".to_string(), Value::String(c.time.to_rfc3339()));
                        Value::Map(m)
                    })
                    .collect();
                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::Array(arr),
                }
            }
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_log failed: {}", e),
            },
        }
    }

    async fn handle_workspace_commit(&self, request_id: u64, params: Value) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let author = match get_string_param(&params, "author") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'author' parameter".into(),
                };
            }
        };
        let message = match get_string_param(&params, "message") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'message' parameter".into(),
                };
            }
        };
        let branch = get_string_param(&params, "branch").unwrap_or_else(|| "develop".into());

        match ws.commit(&ns, &name, &author, &message, &branch) {
            Ok(oid) => {
                let changed_files = ws
                    .changed_files_for_commit(&ns, &name, &branch, &oid)
                    .unwrap_or_default();
                let repo_update_result = if changed_files.is_empty() && !(ns == "system" && name == "stdlib") {
                    let area_path = if branch == "develop" {
                        ws.dev_path(&ns, &name)
                    } else {
                        ws.workspace_path(&ns, &name)
                    };
                    if area_path.join(".meta.yml").exists() {
                        self.send_area_reload(AreaId::new(&ns, &name), area_path.to_string_lossy().to_string()).await
                    } else {
                        Ok(())
                    }
                } else {
                    self.handle_repo_updated(&ns, &name, Some((&branch, &changed_files))).await
                };
                if let Err(e) = repo_update_result {
                    warn!(namespace = %ns, name = %name, branch = %branch, error = %e, "post-commit repo handling failed");
                }
                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::String(oid),
                }
            }
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_commit failed: {}", e),
            },
        }
    }

    async fn handle_workspace_pull(&self, request_id: u64, params: Value) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let branch = get_string_param(&params, "branch").unwrap_or_else(|| "develop".into());

        match ws.pull(&ns, &name, &branch) {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_pull failed: {}", e),
            },
        }
    }

    async fn handle_workspace_checkout(&self, request_id: u64, params: Value) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };

        match ws.checkout(&ns, &name) {
            Ok(_) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_checkout failed: {}", e),
            },
        }
    }

    async fn handle_workspace_checkout_branch(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let branch = match get_string_param(&params, "branch") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'branch' parameter".into(),
                };
            }
        };

        match ws.checkout_branch(&ns, &name, &branch) {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_checkout_branch failed: {}", e),
            },
        }
    }

    async fn handle_workspace_branches(&self, request_id: u64, params: Value) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };

        match ws.branches(&ns, &name) {
            Ok(branches) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Array(branches.into_iter().map(Value::String).collect()),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_branches failed: {}", e),
            },
        }
    }

    async fn handle_workspace_create_branch(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
        let ws = match &self.workspace {
            Some(ws) => ws,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "workspace not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let branch_name = match get_string_param(&params, "branch_name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'branch_name' parameter".into(),
                };
            }
        };

        match ws.create_branch(&ns, &name, &branch_name) {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("workspace_create_branch failed: {}", e),
            },
        }
    }

    // -----------------------------------------------------------------
    // Merge request handlers
    // -----------------------------------------------------------------

    async fn handle_mr_create(&self, request_id: u64, params: Value) -> DriverMessage {
        let mrm = match &self.merge_request_manager {
            Some(mrm) => mrm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "merge_request_manager not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let area_name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let author = match get_string_param(&params, "author") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'author' parameter".into(),
                };
            }
        };
        let title = match get_string_param(&params, "title") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'title' parameter".into(),
                };
            }
        };
        let description = get_string_param(&params, "description");
        let source_branch =
            get_string_param(&params, "source_branch").unwrap_or_else(|| "develop".into());
        let target_branch =
            get_string_param(&params, "target_branch").unwrap_or_else(|| "main".into());

        use crate::persistence::merge_request_store::CreateMrParams;
        let create_params = CreateMrParams {
            namespace: ns,
            area_name,
            author,
            title,
            description,
            source_branch,
            target_branch,
        };

        match mrm.create_merge_request(create_params).await {
            Ok(mr) => DriverMessage::RequestResponse {
                request_id,
                result: mr_to_value(&mr),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("mr_create failed: {}", e),
            },
        }
    }

    async fn handle_mr_get(&self, request_id: u64, params: Value) -> DriverMessage {
        let mrm = match &self.merge_request_manager {
            Some(mrm) => mrm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "merge_request_manager not configured".into(),
                };
            }
        };
        let id = match get_int_param(&params, "id") {
            Some(v) => v as i32,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'id' parameter".into(),
                };
            }
        };

        match mrm.get(id).await {
            Ok(Some(mr)) => DriverMessage::RequestResponse {
                request_id,
                result: mr_to_value(&mr),
            },
            Ok(None) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Null,
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("mr_get failed: {}", e),
            },
        }
    }

    async fn handle_mr_list_all(&self, request_id: u64, params: Value) -> DriverMessage {
        let mrm = match &self.merge_request_manager {
            Some(mrm) => mrm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "merge_request_manager not configured".into(),
                };
            }
        };
        let ns = match get_string_param(&params, "namespace") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'namespace' parameter".into(),
                };
            }
        };
        let area_name = match get_string_param(&params, "name") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'name' parameter".into(),
                };
            }
        };
        let state = get_string_param(&params, "state");

        match mrm.list(&ns, &area_name, state.as_deref()).await {
            Ok(mrs) => {
                let arr: Vec<Value> = mrs.iter().map(mr_to_value).collect();
                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::Array(arr),
                }
            }
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("mr_list_all failed: {}", e),
            },
        }
    }

    async fn handle_mr_approve(&self, request_id: u64, params: Value) -> DriverMessage {
        let mrm = match &self.merge_request_manager {
            Some(mrm) => mrm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "merge_request_manager not configured".into(),
                };
            }
        };
        let id = match get_int_param(&params, "id") {
            Some(v) => v as i32,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'id' parameter".into(),
                };
            }
        };
        let username = match get_string_param(&params, "username") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'username' parameter".into(),
                };
            }
        };
        let comment = get_string_param(&params, "comment");

        match mrm.add_approval(id, &username, comment.as_deref()).await {
            Ok(approval) => {
                let mut m = HashMap::new();
                m.insert("id".to_string(), Value::Int(approval.id as i64));
                m.insert(
                    "merge_request_id".to_string(),
                    Value::Int(approval.merge_request_id as i64),
                );
                m.insert("approver".to_string(), Value::String(approval.approver));
                m.insert(
                    "comment".to_string(),
                    approval.comment.map(Value::String).unwrap_or(Value::Null),
                );
                DriverMessage::RequestResponse {
                    request_id,
                    result: Value::Map(m),
                }
            }
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("mr_approve failed: {}", e),
            },
        }
    }

    async fn handle_mr_reject(&self, request_id: u64, params: Value) -> DriverMessage {
        let mrm = match &self.merge_request_manager {
            Some(mrm) => mrm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "merge_request_manager not configured".into(),
                };
            }
        };
        let id = match get_int_param(&params, "id") {
            Some(v) => v as i32,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'id' parameter".into(),
                };
            }
        };
        let username = match get_string_param(&params, "username") {
            Some(v) => v,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'username' parameter".into(),
                };
            }
        };
        let reason = get_string_param(&params, "reason").unwrap_or_default();

        match mrm.reject(id, &username, &reason).await {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("mr_reject failed: {}", e),
            },
        }
    }

    async fn handle_mr_merge(&self, request_id: u64, params: Value) -> DriverMessage {
        let mrm = match &self.merge_request_manager {
            Some(mrm) => mrm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "merge_request_manager not configured".into(),
                };
            }
        };
        let id = match get_int_param(&params, "id") {
            Some(v) => v as i32,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'id' parameter".into(),
                };
            }
        };

        match mrm.execute_merge(id).await {
            Ok(mr) => DriverMessage::RequestResponse {
                request_id,
                result: mr_to_value(&mr),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("mr_merge failed: {}", e),
            },
        }
    }

    async fn handle_mr_close(&self, request_id: u64, params: Value) -> DriverMessage {
        let mrm = match &self.merge_request_manager {
            Some(mrm) => mrm,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "merge_request_manager not configured".into(),
                };
            }
        };
        let id = match get_int_param(&params, "id") {
            Some(v) => v as i32,
            None => {
                return DriverMessage::RequestError {
                    request_id,
                    error: "missing 'id' parameter".into(),
                };
            }
        };

        match mrm.close(id).await {
            Ok(()) => DriverMessage::RequestResponse {
                request_id,
                result: Value::Bool(true),
            },
            Err(e) => DriverMessage::RequestError {
                request_id,
                error: format!("mr_close failed: {}", e),
            },
        }
    }

    /// Append a JSONL entry to the master driver log file.
    fn append_master_log(path: &std::path::Path, level: &str, message: &str, area: Option<&str>) {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let entry = serde_json::json!({
            "ts": ts,
            "level": level,
            "area": area,
            "message": message,
        });
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{}", entry);
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: convert a MergeRequest to a Value::Map
// ---------------------------------------------------------------------------

fn mr_to_value(mr: &crate::persistence::merge_request_store::MergeRequest) -> Value {
    let mut m = HashMap::new();
    m.insert("id".to_string(), Value::Int(mr.id as i64));
    m.insert("namespace".to_string(), Value::String(mr.namespace.clone()));
    m.insert("area_name".to_string(), Value::String(mr.area_name.clone()));
    m.insert("title".to_string(), Value::String(mr.title.clone()));
    m.insert(
        "description".to_string(),
        mr.description
            .as_ref()
            .map(|d| Value::String(d.clone()))
            .unwrap_or(Value::Null),
    );
    m.insert("author".to_string(), Value::String(mr.author.clone()));
    m.insert("state".to_string(), Value::String(mr.state.clone()));
    m.insert(
        "source_branch".to_string(),
        Value::String(mr.source_branch.clone()),
    );
    m.insert(
        "target_branch".to_string(),
        Value::String(mr.target_branch.clone()),
    );
    m.insert(
        "created_at".to_string(),
        Value::String(mr.created_at.to_rfc3339()),
    );
    m.insert(
        "updated_at".to_string(),
        Value::String(mr.updated_at.to_rfc3339()),
    );
    Value::Map(m)
}

// ---------------------------------------------------------------------------
// Helper: replace request_id in a DriverMessage variant
// ---------------------------------------------------------------------------

/// Replace the `request_id` field in a [`DriverMessage`] variant with a new value.
///
/// This is used by the MOP RPC infrastructure to assign unique request IDs
/// to outgoing messages so responses can be routed back to the correct caller.
fn replace_request_id(msg: DriverMessage, new_id: u64) -> DriverMessage {
    match msg {
        DriverMessage::CheckBuilderAccess {
            user,
            namespace,
            area,
            action,
            ..
        } => DriverMessage::CheckBuilderAccess {
            request_id: new_id,
            user,
            namespace,
            area,
            action,
        },
        DriverMessage::CheckRepoAccess {
            username,
            namespace,
            name,
            level,
            ..
        } => DriverMessage::CheckRepoAccess {
            request_id: new_id,
            username,
            namespace,
            name,
            level,
        },
        DriverMessage::GetWebData { area_key, .. } => DriverMessage::GetWebData {
            request_id: new_id,
            area_key,
        },
        // For other variants that have a request_id, add arms here as needed.
        // Variants without request_id pass through unchanged.
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Helper: extract a string param from a Value::Map
// ---------------------------------------------------------------------------

fn get_string_param(params: &Value, key: &str) -> Option<String> {
    match params {
        Value::Map(map) => match map.get(key) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn get_int_param(params: &Value, key: &str) -> Option<i64> {
    match params {
        Value::Map(map) => match map.get(key) {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}

fn classify_stdlib_subsystem(paths: &[String]) -> &'static str {
    let mut world_changed = false;
    let mut portal_changed = false;
    let mut shared_changed = false;

    for path in paths {
        if path.starts_with("portal/") {
            portal_changed = true;
        } else if path.starts_with("world/") || path.starts_with("commands/") {
            world_changed = true;
        } else {
            shared_changed = true;
        }
    }

    if shared_changed || (world_changed && portal_changed) {
        "all"
    } else if portal_changed {
        "portal"
    } else if world_changed {
        "world"
    } else {
        "all"
    }
}

fn stdlib_program_path(subsystem: &str) -> &'static str {
    match subsystem {
        "shared" => "stdlib:ruby:shared",
        "room" => "stdlib:ruby:world:room",
        "item" => "stdlib:ruby:world:item",
        "npc" => "stdlib:ruby:world:npc",
        "daemon" => "stdlib:ruby:world:daemon",
        "area" => "stdlib:ruby:world:area",
        "commands" => "stdlib:ruby:world:commands",
        "area_web" => "stdlib:ruby:world:area_web",
        "portal" => "stdlib:ruby:portal",
        _ => "stdlib:ruby:shared",
    }
}

fn ruby_stdlib_dependency_roots() -> Vec<(&'static str, Vec<String>)> {
    vec![
        (stdlib_program_path("shared"), vec![]),
        (
            stdlib_program_path("room"),
            vec![stdlib_program_path("shared").to_string()],
        ),
        (
            stdlib_program_path("item"),
            vec![stdlib_program_path("shared").to_string()],
        ),
        (
            stdlib_program_path("npc"),
            vec![stdlib_program_path("shared").to_string()],
        ),
        (
            stdlib_program_path("daemon"),
            vec![stdlib_program_path("shared").to_string()],
        ),
        (
            stdlib_program_path("area"),
            vec![stdlib_program_path("shared").to_string()],
        ),
        (
            stdlib_program_path("commands"),
            vec![stdlib_program_path("shared").to_string()],
        ),
        (
            stdlib_program_path("area_web"),
            vec![stdlib_program_path("shared").to_string()],
        ),
        (
            stdlib_program_path("portal"),
            vec![stdlib_program_path("shared").to_string()],
        ),
    ]
}

fn stdlib_roots_for_paths(paths: &[String]) -> std::collections::BTreeSet<&'static str> {
    let mut roots = std::collections::BTreeSet::new();

    for path in paths {
        match path.as_str() {
            "world/room.rb" => {
                roots.insert(stdlib_program_path("room"));
            }
            "world/item.rb" => {
                roots.insert(stdlib_program_path("item"));
            }
            "world/npc.rb" => {
                roots.insert(stdlib_program_path("npc"));
            }
            "world/daemon.rb" => {
                roots.insert(stdlib_program_path("daemon"));
            }
            "world/area.rb" => {
                roots.insert(stdlib_program_path("area"));
            }
            "world/review_policy.rb" | "commands/command.rb" | "commands/parser.rb"
            | "commands/builder.rb" => {
                roots.insert(stdlib_program_path("commands"));
            }
            "world/web_data_helpers.rb" => {
                roots.insert(stdlib_program_path("area_web"));
            }
            "web/rack_app.rb" => {
                roots.insert(stdlib_program_path("portal"));
            }
            _ if path.starts_with("portal/") => {
                roots.insert(stdlib_program_path("portal"));
            }
            _ if path.starts_with("templates/") => {}
            _ => {
                roots.insert(stdlib_program_path("shared"));
            }
        }
    }

    roots
}

fn affected_stdlib_roots(paths: &[String]) -> Vec<&'static str> {
    let roots = stdlib_roots_for_paths(paths);
    if roots.contains(stdlib_program_path("shared")) {
        return ruby_stdlib_dependency_roots()
            .into_iter()
            .map(|(program, _)| program)
            .collect();
    }
    roots.into_iter().collect()
}

fn area_program_path(area_id: &AreaId) -> String {
    format!("area:{}/{}", area_id.namespace, area_id.name)
}

fn parse_area_program_path(program: &str) -> Option<AreaId> {
    let area = program.strip_prefix("area:")?;
    let (namespace, name) = area.split_once('/')?;
    Some(AreaId::new(namespace, name))
}

fn area_dependency_paths(area_path: &str, language: &str) -> Vec<String> {
    if language != "ruby" {
        return Vec::new();
    }

    let area_path = Path::new(area_path);
    let mut deps = std::collections::BTreeSet::new();
    deps.insert(stdlib_program_path("shared").to_string());
    deps.insert(stdlib_program_path("area").to_string());

    if area_path.join("rooms").exists() {
        deps.insert(stdlib_program_path("room").to_string());
    }
    if area_path.join("items").exists() {
        deps.insert(stdlib_program_path("item").to_string());
    }
    if area_path.join("npcs").exists() {
        deps.insert(stdlib_program_path("npc").to_string());
    }
    if area_path.join("daemons").exists() {
        deps.insert(stdlib_program_path("daemon").to_string());
    }
    if area_path.join("commands").exists() {
        deps.insert(stdlib_program_path("commands").to_string());
    }
    if area_path.join("mud_web.rb").exists() || area_path.join("web").exists() {
        deps.insert(stdlib_program_path("area_web").to_string());
    }

    for file in collect_ruby_files(area_path) {
        if let Ok(source) = std::fs::read_to_string(&file) {
            if source.contains("< Room") || source.contains("Room.new") {
                deps.insert(stdlib_program_path("room").to_string());
            }
            if source.contains("< Item") || source.contains("Item.new") {
                deps.insert(stdlib_program_path("item").to_string());
            }
            if source.contains("< NPC") || source.contains("NPC.new") {
                deps.insert(stdlib_program_path("npc").to_string());
            }
            if source.contains("< Daemon") || source.contains("Daemon.new") {
                deps.insert(stdlib_program_path("daemon").to_string());
            }
            if source.contains("web_data do") || source.contains("render_template(") {
                deps.insert(stdlib_program_path("area_web").to_string());
            }
            if source.contains("Command") || source.contains("Parser") {
                deps.insert(stdlib_program_path("commands").to_string());
            }
        }
    }

    deps.into_iter().collect()
}

fn collect_ruby_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_ruby_files(&path));
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rb") {
                files.push(path);
            }
        }
    }
    files
}

// ---------------------------------------------------------------------------
// Area discovery — public for testability
// ---------------------------------------------------------------------------

/// Discover areas in the given world directory.
///
/// Scans for directories matching `world_path/<namespace>/<name>/` and
/// returns those that:
/// Recursively collect all files under `dir` into a `HashMap<String, String>`
/// where keys are relative paths and values are file contents (UTF-8).
fn collect_template_files(dir: &std::path::Path) -> Result<HashMap<String, String>> {
    let mut files = HashMap::new();
    collect_files_recursive(dir, dir, &mut files)?;
    Ok(files)
}

fn scan_disk_template_by_name(name: &str) -> Result<HashMap<String, String>> {
    match name {
        "default" => collect_template_files(Path::new("bootstrap/ruby/stdlib/templates/area")),
        "lpc" => collect_template_files(Path::new("bootstrap/lpc/templates/area/lpc")),
        "rust" => collect_template_files(Path::new("bootstrap/lpc/templates/area/rust")),
        template if template.starts_with("kotlin:") => {
            let overlay = template.trim_start_matches("kotlin:");
            let base = collect_template_files(Path::new("bootstrap/jvm/templates/area/base"))?;
            let overlay_files = collect_template_files(
                Path::new("bootstrap/jvm/templates/area/overlays")
                    .join(overlay)
                    .as_path(),
            )?;
            let mut merged = base;
            merged.extend(overlay_files);
            Ok(merged)
        }
        other => Err(anyhow::anyhow!("unknown bootstrap template '{}'", other)),
    }
}

fn template_repo_name(template_name: &str) -> String {
    let normalized = template_name
        .to_ascii_lowercase()
        .replace(':', "_")
        .replace('-', "_");
    format!("template_{normalized}")
}

fn default_template_metadata(template_name: &str) -> TemplateMetadata {
    match template_name {
        "default" => TemplateMetadata {
            name: "default".into(),
            kind: "area_template".into(),
            language: "ruby".into(),
            framework: None,
            display_name: Some("Ruby Default".into()),
            description: Some("Default Ruby area template".into()),
            branch: Some("main".into()),
            stdlib_compatible: Some(true),
        },
        "lpc" => TemplateMetadata {
            name: "lpc".into(),
            kind: "area_template".into(),
            language: "lpc".into(),
            framework: None,
            display_name: Some("LPC".into()),
            description: Some("LPC area template".into()),
            branch: Some("main".into()),
            stdlib_compatible: Some(true),
        },
        "rust" => TemplateMetadata {
            name: "rust".into(),
            kind: "area_template".into(),
            language: "rust".into(),
            framework: None,
            display_name: Some("Rust".into()),
            description: Some("Rust area template".into()),
            branch: Some("main".into()),
            stdlib_compatible: Some(true),
        },
        "kotlin:ktor" => TemplateMetadata {
            name: "kotlin:ktor".into(),
            kind: "area_template".into(),
            language: "kotlin".into(),
            framework: Some("ktor".into()),
            display_name: Some("Kotlin Ktor".into()),
            description: Some("Kotlin template using Ktor".into()),
            branch: Some("main".into()),
            stdlib_compatible: Some(true),
        },
        "kotlin:quarkus" => TemplateMetadata {
            name: "kotlin:quarkus".into(),
            kind: "area_template".into(),
            language: "kotlin".into(),
            framework: Some("quarkus".into()),
            display_name: Some("Kotlin Quarkus".into()),
            description: Some("Kotlin template using Quarkus".into()),
            branch: Some("main".into()),
            stdlib_compatible: Some(true),
        },
        "kotlin:spring-boot" => TemplateMetadata {
            name: "kotlin:spring-boot".into(),
            kind: "area_template".into(),
            language: "kotlin".into(),
            framework: Some("spring-boot".into()),
            display_name: Some("Kotlin Spring Boot".into()),
            description: Some("Kotlin template using Spring Boot".into()),
            branch: Some("main".into()),
            stdlib_compatible: Some(true),
        },
        other => TemplateMetadata {
            name: other.into(),
            kind: "area_template".into(),
            language: "unknown".into(),
            framework: None,
            display_name: None,
            description: None,
            branch: Some("main".into()),
            stdlib_compatible: None,
        },
    }
}

fn with_template_metadata_file(
    template_name: &str,
    mut files: HashMap<String, String>,
) -> HashMap<String, String> {
    if !files.contains_key("template.yml") {
        let metadata = default_template_metadata(template_name);
        if let Ok(yaml) = serde_yaml::to_string(&metadata) {
            files.insert("template.yml".into(), yaml);
        }
    }
    files
}

fn with_stdlib_metadata_file(mut files: HashMap<String, String>) -> HashMap<String, String> {
    if !files.contains_key(".meta.yml") {
        files.insert(".meta.yml".into(), "owner: system\nsystem: true\n".into());
    }
    files
}

fn collect_files_recursive(
    root: &std::path::Path,
    current: &std::path::Path,
    files: &mut HashMap<String, String>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(current).with_context(|| format!("reading dir: {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, files)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            // Skip .gitkeep and other dotfiles
            if rel.contains("/.gitkeep") || rel == ".gitkeep" {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                files.insert(rel, content);
            }
        }
    }
    Ok(())
}

/// - Are directories (not files)
/// - Do not contain `@dev` in their name
/// - Contain a `.meta.yml` file
///
/// Returns a sorted list of `(AreaId, path_string)` pairs.
pub fn discover_areas(world_path: &str) -> Result<Vec<(AreaId, String)>> {
    let pattern = format!("{world_path}/*/*");

    let entries =
        glob::glob(&pattern).with_context(|| format!("invalid glob pattern: {pattern}"))?;

    let mut areas = Vec::new();

    for entry in entries {
        let path = match entry {
            Ok(p) => p,
            Err(e) => {
                warn!(%e, "glob error, skipping entry");
                continue;
            }
        };

        // Only directories are area candidates.
        if !path.is_dir() {
            continue;
        }

        // Skip @dev checkouts.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.contains("@dev") {
                continue;
            }
        }

        // Skip directories without a .meta.yml file.
        if !path.join(".meta.yml").exists() {
            continue;
        }

        // Extract namespace/name from the path:  world/<namespace>/<name>
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let namespace = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        if namespace.is_empty() || name.is_empty() {
            warn!(path = %path.display(), "could not extract namespace/name, skipping");
            continue;
        }

        if namespace == "system" && name.starts_with("template_") {
            continue;
        }

        let area_id = AreaId::new(&namespace, &name);
        let area_path = path.to_string_lossy().to_string();

        areas.push((area_id, area_path));
    }

    // Sort by area_id for deterministic ordering.
    areas.sort_by(|a, b| a.0.to_string().cmp(&b.0.to_string()));

    Ok(areas)
}

/// Load encryption key from `<data_dir>/encryption.key`, or generate and save one.
///
/// The key is stored as a 64-character hex string (32 bytes). This allows the
/// AI key store to work without manual configuration.
fn load_or_generate_encryption_key(data_path: &str) -> Result<String> {
    let key_path = std::path::Path::new(data_path).join("encryption.key");

    if key_path.exists() {
        let contents = std::fs::read_to_string(&key_path)
            .with_context(|| format!("reading encryption key from {}", key_path.display()))?;
        let hex_key = contents.trim().to_string();
        if hex_key.len() == 64 && hex::decode(&hex_key).is_ok() {
            info!("Loaded encryption key from {}", key_path.display());
            return Ok(hex_key);
        }
        warn!(
            "Invalid encryption key in {} (expected 64 hex chars), generating new one",
            key_path.display()
        );
    }

    // Generate a new 32-byte random key
    let mut key_bytes = [0u8; 32];
    rand::fill(&mut key_bytes);
    let hex_key = hex::encode(key_bytes);

    std::fs::write(&key_path, &hex_key)
        .with_context(|| format!("writing encryption key to {}", key_path.display()))?;
    info!("Generated new encryption key at {}", key_path.display());

    Ok(hex_key)
}

fn enabled_adapter_count(config: &Config) -> u32 {
    let mut count = 0u32;
    if config.adapters.ruby.as_ref().is_some_and(|r| r.enabled) {
        count += 1;
    }
    if config.adapters.jvm.as_ref().is_some_and(|j| j.enabled) {
        count += 1;
    }
    if config.adapters.lpc.as_ref().is_some_and(|l| l.enabled) {
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{JvmAdapterConfig, LpcAdapterConfig, RubyAdapterConfig};

    // -- SessionState tests --

    #[test]
    fn session_state_first_id_is_1() {
        let mut state = SessionState::new();
        let (tx, _rx) = mpsc::channel(1);
        let id = state.allocate_session(tx);
        assert_eq!(id, 1);
    }

    #[test]
    fn session_state_ids_increment() {
        let mut state = SessionState::new();
        let ids: Vec<u64> = (0..5)
            .map(|_| {
                let (tx, _rx) = mpsc::channel(1);
                state.allocate_session(tx)
            })
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn session_state_remove_session() {
        let mut state = SessionState::new();
        let (tx, _rx) = mpsc::channel(1);
        let id = state.allocate_session(tx);
        state.remove_session(id);
        // After removal, send_output should not panic
        state.send_output(id, "test".into());
    }

    #[test]
    fn session_state_remove_nonexistent_no_panic() {
        let mut state = SessionState::new();
        state.remove_session(999); // Should not panic
    }

    #[test]
    fn session_state_send_output_to_valid_session() {
        let mut state = SessionState::new();
        let (tx, mut rx) = mpsc::channel(64);
        let id = state.allocate_session(tx);
        state.send_output(id, "hello".into());
        let msg = rx.try_recv().unwrap();
        assert_eq!(msg, "hello");
    }

    #[test]
    fn session_state_send_output_to_unknown_session() {
        let state = SessionState::new();
        // Should not panic, just log a warning
        state.send_output(999, "hello".into());
    }

    #[test]
    fn session_state_send_output_after_removal() {
        let mut state = SessionState::new();
        let (tx, _rx) = mpsc::channel(1);
        let id = state.allocate_session(tx);
        state.remove_session(id);
        // Should not panic
        state.send_output(id, "hello".into());
    }

    #[test]
    fn session_state_multiple_sessions() {
        let mut state = SessionState::new();
        let (tx1, mut rx1) = mpsc::channel(64);
        let (tx2, mut rx2) = mpsc::channel(64);
        let id1 = state.allocate_session(tx1);
        let id2 = state.allocate_session(tx2);

        state.send_output(id1, "msg1".into());
        state.send_output(id2, "msg2".into());

        assert_eq!(rx1.try_recv().unwrap(), "msg1");
        assert_eq!(rx2.try_recv().unwrap(), "msg2");
    }

    #[test]
    fn enabled_adapter_count_includes_lpc() {
        let mut config = Config::default();
        config.adapters.ruby = Some(RubyAdapterConfig {
            enabled: true,
            ..RubyAdapterConfig::default()
        });
        config.adapters.jvm = Some(JvmAdapterConfig {
            enabled: true,
            ..JvmAdapterConfig::default()
        });
        config.adapters.lpc = Some(LpcAdapterConfig {
            enabled: true,
            ..LpcAdapterConfig::default()
        });

        assert_eq!(enabled_adapter_count(&config), 3);
    }

    #[test]
    fn template_repo_name_normalizes_template_names() {
        assert_eq!(template_repo_name("default"), "template_default");
        assert_eq!(template_repo_name("kotlin:ktor"), "template_kotlin_ktor");
        assert_eq!(
            template_repo_name("kotlin:spring-boot"),
            "template_kotlin_spring_boot"
        );
    }

    #[test]
    fn metadata_file_is_injected_for_bootstrap_templates() {
        let files = with_template_metadata_file("lpc", HashMap::new());
        let metadata = files.get("template.yml").expect("template metadata");
        assert!(metadata.contains("name: lpc"));
        assert!(metadata.contains("kind: area_template"));
        assert!(metadata.contains("language: lpc"));
    }

    #[test]
    fn metadata_file_is_injected_for_bootstrap_stdlib() {
        let files = with_stdlib_metadata_file(HashMap::new());
        let metadata = files.get(".meta.yml").expect("stdlib metadata");
        assert!(metadata.contains("owner: system"));
        assert!(metadata.contains("system: true"));
    }

    #[test]
    fn parse_area_program_path_round_trip() {
        let area_id = AreaId::new("vikings", "village");
        let program = area_program_path(&area_id);
        let parsed = parse_area_program_path(&program).expect("parsed area id");
        assert_eq!(parsed.namespace, "vikings");
        assert_eq!(parsed.name, "village");
    }

    #[test]
    fn affected_stdlib_roots_map_changed_files() {
        let room = affected_stdlib_roots(&["world/room.rb".to_string()]);
        assert_eq!(room, vec![stdlib_program_path("room")]);

        let portal = affected_stdlib_roots(&["portal/app.rb".to_string()]);
        assert_eq!(portal, vec![stdlib_program_path("portal")]);

        let all = affected_stdlib_roots(&["system/access_control.rb".to_string()]);
        assert!(all.contains(&stdlib_program_path("shared")));
        assert!(all.contains(&stdlib_program_path("room")));
        assert!(all.contains(&stdlib_program_path("portal")));
    }

    #[test]
    fn ruby_area_dependencies_include_precise_runtime_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let area_path = dir.path().join("area");
        std::fs::create_dir_all(area_path.join("rooms")).expect("create rooms dir");
        std::fs::create_dir_all(area_path.join("items")).expect("create items dir");
        std::fs::create_dir_all(area_path.join("web")).expect("create web dir");
        std::fs::write(area_path.join("mud_web.rb"), "# web").expect("write mud_web.rb");

        let deps = area_dependency_paths(&area_path.to_string_lossy(), "ruby");
        assert!(deps.contains(&stdlib_program_path("shared").to_string()));
        assert!(deps.contains(&stdlib_program_path("area").to_string()));
        assert!(deps.contains(&stdlib_program_path("room").to_string()));
        assert!(deps.contains(&stdlib_program_path("item").to_string()));
        assert!(deps.contains(&stdlib_program_path("area_web").to_string()));
        assert!(!deps.contains(&stdlib_program_path("portal").to_string()));
    }

    #[test]
    fn non_ruby_area_dependencies_are_empty_for_now() {
        assert!(area_dependency_paths("/tmp/example", "lpc").is_empty());
        assert!(area_dependency_paths("/tmp/example", "rust").is_empty());
    }
}
