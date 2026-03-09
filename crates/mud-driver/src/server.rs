use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use mud_core::types::AreaId;
use mud_mop::message::{AdapterMessage, DriverMessage, Value};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Thread-safe, shared area template registry.
/// Outer key is template name (e.g. "default", "kotlin:ktor"), inner keys
/// are file paths with `{{namespace}}` / `{{area_name}}` placeholders.
pub type AreaTemplates = Arc<RwLock<HashMap<String, HashMap<String, String>>>>;

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
use crate::ssh::handler::SshCommand;
use crate::ssh::server::start_ssh_server;
use crate::web::build_log::BuildLog;
use crate::web::build_manager::BuildManager;
use crate::web::server::{AppState, WebServer, init_templates};
use crate::web::skills::SkillsService;

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
    adapter_languages: Vec<String>,
    /// Database manager, initialized when `database.admin_password` is configured.
    #[allow(dead_code)]
    db_manager: Option<DatabaseManager>,
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
        let master_log_path = Some(
            config.world.resolved_path().join(".mud").join("driver.log"),
        );
        let build_log = Arc::new(BuildLog::new(200));
        let build_cache_path = std::path::PathBuf::from(&config.http.build_cache_path);
        let build_manager = Some(Arc::new(BuildManager::new(Arc::clone(&build_log), build_cache_path)));

        // Create the MOP RPC channel for web-handler-to-adapter communication.
        let (mop_rpc_tx, mop_rpc_rx) = mpsc::channel::<MopRequest>(32);
        let mop_rpc_client = MopRpcClient::new(mop_rpc_tx);

        Self {
            config,
            adapter_manager,
            sessions: SessionState::new(),
            adapter_languages: Vec::new(),
            db_manager: None,
            player_store: None,
            repo_manager: None,
            workspace: None,
            merge_request_manager: None,
            area_templates: Arc::new(RwLock::new(HashMap::new())),
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
        let mut expected_adapters = 0u32;
        if self.config.adapters.ruby.as_ref().is_some_and(|r| r.enabled) {
            expected_adapters += 1;
        }
        if self.config.adapters.jvm.as_ref().is_some_and(|j| j.enabled) {
            expected_adapters += 1;
        }

        info!(expected_adapters, "Waiting for adapters (30s timeout)...");

        for i in 0..expected_adapters {
            let language = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                self.adapter_manager.accept_connection(&listener),
            )
            .await
            .map_err(|_| anyhow::anyhow!(
                "timed out waiting for adapter {}/{} to connect (30s). \
                 Check that the adapter binary exists and is configured.",
                i + 1, expected_adapters,
            ))?
            .context("accepting adapter connection")?;

            info!(language, adapter = i + 1, total = expected_adapters, "Adapter connected");
            self.adapter_languages.push(language);
        }

        // -----------------------------------------------------------------
        // Database setup (optional — requires admin_password)
        // -----------------------------------------------------------------
        self.setup_database().await?;

        // -----------------------------------------------------------------
        // Web server (optional — requires http.enabled)
        // -----------------------------------------------------------------
        if self.config.http.enabled {
            if let (Some(ps), Some(rm), Some(ws)) =
                (&self.player_store, &self.repo_manager, &self.workspace)
            {
                let templates = Arc::new(
                    init_templates().context("initializing web templates")?,
                );

                // Build AiKeyStore — auto-generate encryption key if not configured.
                let ai_key_store = if let Some(db_mgr) = &self.db_manager {
                    let key_hex = match &self.config.database.encryption_key {
                        Some(k) => k.clone(),
                        None => load_or_generate_encryption_key(&self.config.world.data_path)?,
                    };
                    let key_bytes = hex::decode(&key_hex)
                        .context("decoding encryption_key as hex")?;
                    let encryptor = Arc::new(CredentialEncryptor::new(&key_bytes)
                        .context("creating credential encryptor")?);
                    info!("AI key store initialized");
                    Some(Arc::new(AiKeyStore::new(
                        db_mgr.driver_pool().clone(),
                        encryptor,
                    )))
                } else {
                    info!("AI key store not configured (no database manager)");
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
                    loaded_areas: Arc::clone(&self.loaded_areas),
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

        let areas = discover_areas(&self.config.world.resolved_path().to_string_lossy())?;

        for (area_id, area_path) in &areas {
            let language = self.language_for_area(area_path);
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
    /// If `mud.yaml` exists and specifies a JVM framework (anything other than
    /// `none`), the area belongs to the "kotlin" adapter. Otherwise it defaults
    /// to the primary language (Ruby).
    fn language_for_area(&self, area_path: &str) -> String {
        let yaml_path = std::path::Path::new(area_path).join("mud.yaml");
        match std::fs::read_to_string(&yaml_path) {
            Ok(contents) => {
                if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&contents) {
                    if let Some(framework) = yaml.get("framework").and_then(|v| v.as_str()) {
                        if framework != "none"
                            && self.adapter_languages.iter().any(|l| l == "kotlin")
                        {
                            info!(path = %area_path, %framework, "Routing area to kotlin adapter");
                            return "kotlin".to_string();
                        }
                        info!(path = %area_path, %framework, "Area uses ruby (framework=none or no kotlin adapter)");
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
    /// adapter language for subsequent session calls.
    pub async fn accept_adapter(
        &mut self,
        listener: &tokio::net::UnixListener,
    ) -> Result<String> {
        let language = self
            .adapter_manager
            .accept_connection(listener)
            .await
            .context("accepting adapter connection")?;
        self.adapter_languages.push(language.clone());
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
    pub async fn send_configure(&self, stdlib_db_url: String) -> Result<()> {
        for lang in &self.adapter_languages {
            self.adapter_manager
                .send_to(lang, DriverMessage::Configure { stdlib_db_url: stdlib_db_url.clone() })
                .await
                .with_context(|| format!("sending configure to {lang} adapter"))?;
        }
        Ok(())
    }

    /// Send a LoadArea message to the appropriate adapter based on area type.
    pub async fn send_load_area(&self, area_id: AreaId, path: String) -> Result<()> {
        if !self.adapter_languages.is_empty() {
            let lang = self.language_for_area(&path);
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
                .send_to(&lang, DriverMessage::LoadArea { area_id, path, db_url })
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

        let db_manager = DatabaseManager::new(&self.config.database)
            .await
            .context("initializing database manager")?;

        // Send stdlib DB URL to the adapter immediately after databases are
        // created so it can run its Sequel migrations concurrently with the
        // driver migrations below.
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
        self.db_manager = Some(db_manager);

        info!("PlayerStore, RepoManager, Workspace, and MergeRequestManager initialized");

        Ok(())
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
    async fn run_event_loop(
        &mut self,
        mut ssh_cmd_rx: mpsc::Receiver<SshCommand>,
    ) -> Result<()> {
        // Take the receiver out of the Option so we can use it in the loop.
        // If it was already taken (shouldn't happen), create a dummy channel.
        let mut mop_rpc_rx = self.mop_rpc_rx.take()
            .unwrap_or_else(|| mpsc::channel::<MopRequest>(1).1);

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
                else => {
                    info!("Event loop: all channels closed, shutting down");
                    break;
                }
            }
        }
        self.adapter_manager.shutdown();
        Ok(())
    }

    /// Handle a MOP RPC request submitted by a web handler.
    ///
    /// Assigns a unique request ID, stores the response channel, and sends
    /// the message to the adapter. The adapter's `CallResult`/`CallError`
    /// response will be routed back via [`complete_rpc`].
    async fn handle_mop_rpc_request(&mut self, rpc_req: MopRequest) {
        let rpc_id = self.next_rpc_id;
        self.next_rpc_id += 1;

        // Replace the request_id in the message with our assigned ID.
        let message = replace_request_id(rpc_req.message, rpc_id);

        // Store the oneshot sender so we can deliver the response later.
        self.pending_rpc.insert(rpc_id, rpc_req.response_tx);

        // Send to the primary adapter (Ruby handles portal RPC).
        let lang = self.primary_language();
        if let Err(e) = self.adapter_manager.send_to(lang, message).await {
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
            } => {
                if !self.complete_rpc(request_id, Ok(result)) {
                    warn!(request_id, "received CallResult for unknown RPC request");
                }
            }
            AdapterMessage::CallError {
                request_id,
                error,
            } => {
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
                let response = self.handle_driver_request(request_id, &action, params).await;
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
            "repo_check_access" => return self.handle_repo_check_access(request_id, params).await,
            "area_reload" => return self.handle_area_reload(request_id, params).await,
            "workspace_diff" => return self.handle_workspace_diff(request_id, params).await,
            "workspace_log" => return self.handle_workspace_log(request_id, params).await,
            "workspace_commit" => return self.handle_workspace_commit(request_id, params).await,
            "workspace_pull" => return self.handle_workspace_pull(request_id, params).await,
            "workspace_checkout" => return self.handle_workspace_checkout(request_id, params).await,
            "workspace_checkout_branch" => return self.handle_workspace_checkout_branch(request_id, params).await,
            "workspace_branches" => return self.handle_workspace_branches(request_id, params).await,
            "workspace_create_branch" => return self.handle_workspace_create_branch(request_id, params).await,
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
            let templates = self.area_templates.read().await;
            let template = self.config.adapters.default_template.as_ref()
                .and_then(|name| templates.get(name))
                .or_else(|| templates.get("default"))
                .or_else(|| templates.values().next());
            if let Err(e) = rm.create_repo(&username, &username, true, template) {
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
    async fn handle_set_area_template(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
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

    /// Register a per-area web socket path for API proxying.
    async fn handle_register_area_web(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
        let area_key = get_string_param(&params, "area_key")
            .unwrap_or_default();
        let socket_path = get_string_param(&params, "socket_path")
            .unwrap_or_default();
        info!(area_key = %area_key, socket_path = %socket_path, "Area web socket registered");
        self.area_web_sockets.write().await.insert(area_key, socket_path);
        DriverMessage::RequestResponse {
            request_id,
            result: Value::Bool(true),
        }
    }

    /// Create a new git repository.
    async fn handle_repo_create(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
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

        let templates = self.area_templates.read().await;
        let template = template_name
            .as_ref()
            .and_then(|name| templates.get(name))
            .or_else(|| templates.get("default"))
            .or_else(|| templates.values().next());

        match rm.create_repo(&ns, &name, seed, template) {
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
    async fn handle_repo_list(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
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
    async fn handle_repo_check_access(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
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

    /// Reload an area by sending ReloadArea to the appropriate adapter.
    async fn handle_area_reload(
        &self,
        request_id: u64,
        params: Value,
    ) -> DriverMessage {
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
                        m.insert(
                            "time".to_string(),
                            Value::String(c.time.to_rfc3339()),
                        );
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
                // After commit succeeds, trigger SPA build if applicable.
                // Use @dev path/key for develop branch so the build cache
                // and vite base URL match the serving URL.
                if let Some(ref build_manager) = self.build_manager {
                    let (area_path, area_key, base_url) = if branch == "develop" {
                        let path = ws.dev_path(&ns, &name);
                        (path, format!("{ns}/{name}@dev"), format!("/project/{ns}/{name}@dev/"))
                    } else {
                        let path = ws.workspace_path(&ns, &name);
                        (path, format!("{ns}/{name}"), format!("/project/{ns}/{name}/"))
                    };
                    if BuildManager::is_spa(&area_path) {
                        build_manager.trigger_build(area_key, area_path, base_url);
                    }
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
                    approval
                        .comment
                        .map(Value::String)
                        .unwrap_or(Value::Null),
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
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
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

// ---------------------------------------------------------------------------
// Area discovery — public for testability
// ---------------------------------------------------------------------------

/// Discover areas in the given world directory.
///
/// Scans for directories matching `world_path/<namespace>/<name>/` and
/// returns those that:
/// - Are directories (not files)
/// - Do not contain `@dev` in their name
/// - Contain a `.meta.yml` file
///
/// Returns a sorted list of `(AreaId, path_string)` pairs.
pub fn discover_areas(world_path: &str) -> Result<Vec<(AreaId, String)>> {
    let pattern = format!("{world_path}/*/*");

    let entries = glob::glob(&pattern)
        .with_context(|| format!("invalid glob pattern: {pattern}"))?;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
