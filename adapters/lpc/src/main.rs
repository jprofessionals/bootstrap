use std::collections::HashMap;
use std::path::Path;

use clap::Parser as ClapParser;
use tokio::io::{BufReader, BufWriter};
use tokio::net::UnixStream;

use lpc_vm::bytecode::LpcValue;
use lpc_vm::vm::Vm;
use mud_core::types::{AreaId, SessionId};
use mud_mop::codec::{read_driver_message, write_adapter_message, CodecError};
use mud_mop::message::{AdapterMessage, DriverMessage, Value};

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

#[derive(ClapParser)]
struct Args {
    adapter_path: String,
    #[arg(long)]
    socket: String,
}

// ---------------------------------------------------------------------------
// Adapter state
// ---------------------------------------------------------------------------

struct AdapterState {
    areas: HashMap<String, AreaState>,
    sessions: HashMap<SessionId, SessionState>,
    vm: Vm,
    #[allow(dead_code)]
    stdlib_db_url: Option<String>,
}

struct AreaState {
    #[allow(dead_code)]
    area_id: AreaId,
    #[allow(dead_code)]
    path: String,
    #[allow(dead_code)]
    db_url: Option<String>,
    /// The object paths loaded for this area (relative to area root).
    loaded_objects: Vec<String>,
}

struct SessionState {
    #[allow(dead_code)]
    session_id: SessionId,
    username: String,
    area_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("mud_adapter_lpc=debug,lpc_vm=debug")
        .init();

    let args = Args::parse();

    tracing::info!("connecting to driver at {}", args.socket);
    let stream = UnixStream::connect(&args.socket).await?;
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // Send handshake — declare both "lpc" and "rust" languages so the
    // driver routes both area types to this adapter.
    let handshake = AdapterMessage::Handshake {
        adapter_name: "mud-adapter-lpc".into(),
        language: "lpc".into(),
        version: "0.1.0".into(),
        languages: vec!["lpc".into(), "rust".into()],
    };
    write_adapter_message(&mut writer, &handshake).await?;
    tracing::info!("handshake sent");
    let mut state = AdapterState {
        areas: HashMap::new(),
        sessions: HashMap::new(),
        vm: Vm::new(),
        stdlib_db_url: None,
    };

    // Main event loop
    loop {
        match read_driver_message(&mut reader).await {
            Ok(msg) => {
                let responses = handle_message(&mut state, msg).await;
                for response in responses {
                    write_adapter_message(&mut writer, &response).await?;
                }
            }
            Err(CodecError::Closed) => {
                tracing::info!("driver connection closed");
                break;
            }
            Err(e) => {
                tracing::error!("MOP read error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Message handler
// ---------------------------------------------------------------------------

async fn handle_message(state: &mut AdapterState, msg: DriverMessage) -> Vec<AdapterMessage> {
    match msg {
        DriverMessage::Configure { stdlib_db_url } => {
            tracing::info!("configured with stdlib_db_url");
            state.stdlib_db_url = Some(stdlib_db_url);
            vec![]
        }

        DriverMessage::LoadArea {
            area_id,
            path,
            db_url,
        } => {
            tracing::info!("loading area {} from {}", area_id, path);
            vec![load_area(state, area_id, &path, db_url)]
        }

        DriverMessage::ReloadArea {
            area_id,
            path,
            db_url,
        } => {
            tracing::info!("reloading area {} from {}", area_id, path);
            // Unload first, then reload
            let key = area_id.key();
            if state.areas.contains_key(&key) {
                unload_area_internal(state, &key);
            }
            vec![load_area(state, area_id, &path, db_url)]
        }

        DriverMessage::UnloadArea { area_id } => {
            let key = area_id.key();
            tracing::info!("unloading area {}", key);
            unload_area_internal(state, &key);
            vec![]
        }

        DriverMessage::ReloadProgram {
            area_id,
            path,
            files,
        } => {
            tracing::info!("reloading {} files in area {}", files.len(), area_id);
            reload_programs(state, &area_id, &path, &files)
        }

        DriverMessage::SessionStart {
            session_id,
            username,
        } => {
            tracing::info!("session start: {} for {}", session_id, username);
            handle_session_start(state, session_id, username)
        }

        DriverMessage::SessionInput { session_id, line } => {
            tracing::debug!("session input: {} -> {:?}", session_id, line);
            handle_session_input(state, session_id, &line)
        }

        DriverMessage::SessionEnd { session_id } => {
            tracing::info!("session end: {}", session_id);
            handle_session_end(state, session_id)
        }

        DriverMessage::Call {
            request_id,
            object_id,
            method,
            args,
        } => {
            tracing::debug!(
                "call: request_id={} object_id={} method={}",
                request_id,
                object_id,
                method
            );
            vec![handle_call(state, request_id, object_id, &method, &args)]
        }

        DriverMessage::Ping { seq } => {
            vec![AdapterMessage::Pong { seq }]
        }

        DriverMessage::CheckBuilderAccess {
            request_id,
            user,
            namespace,
            area,
            action,
        } => {
            tracing::debug!(
                "check_builder_access: user={} ns={} area={} action={}",
                user,
                namespace,
                area,
                action
            );
            // Allow all access for now
            let result = Value::Map(HashMap::from([("allowed".into(), Value::Bool(true))]));
            vec![AdapterMessage::CallResult {
                request_id,
                result,
                cache: None,
            }]
        }

        DriverMessage::CheckRepoAccess {
            request_id,
            username,
            namespace,
            name,
            level,
        } => {
            tracing::debug!(
                "check_repo_access: user={} repo={}/{} level={}",
                username,
                namespace,
                name,
                level
            );
            let result = Value::Map(HashMap::from([("allowed".into(), Value::Bool(false))]));
            vec![AdapterMessage::CallResult {
                request_id,
                result,
                cache: None,
            }]
        }

        DriverMessage::ReloadStdlib { subsystem } => {
            tracing::info!("ignoring stdlib reload for subsystem {}", subsystem);
            vec![]
        }

        DriverMessage::GetWebData {
            request_id,
            area_key,
        } => {
            tracing::debug!("get_web_data for area {}", area_key);
            vec![handle_get_web_data(state, request_id, &area_key)]
        }

        DriverMessage::RequestResponse { request_id, result } => {
            tracing::debug!(
                "received request_response for id={}: {:?}",
                request_id,
                result
            );
            vec![]
        }

        DriverMessage::RequestError { request_id, error } => {
            tracing::warn!("received request_error for id={}: {}", request_id, error);
            vec![]
        }
    }
}

// ---------------------------------------------------------------------------
// Area loading
// ---------------------------------------------------------------------------

fn load_area(
    state: &mut AdapterState,
    area_id: AreaId,
    path: &str,
    db_url: Option<String>,
) -> AdapterMessage {
    let key = area_id.key();

    match load_area_internal(state, &area_id, path, db_url.clone()) {
        Ok(loaded_objects) => {
            state.areas.insert(
                key,
                AreaState {
                    area_id: area_id.clone(),
                    path: path.to_string(),
                    db_url,
                    loaded_objects,
                },
            );
            AdapterMessage::AreaLoaded { area_id }
        }
        Err(e) => {
            tracing::error!("failed to load area {}: {}", area_id, e);
            AdapterMessage::AreaError {
                area_id,
                error: e.to_string(),
            }
        }
    }
}

fn load_area_internal(
    state: &mut AdapterState,
    area_id: &AreaId,
    path: &str,
    _db_url: Option<String>,
) -> anyhow::Result<Vec<String>> {
    let area_path = Path::new(path);
    if !area_path.exists() {
        anyhow::bail!("area path does not exist: {}", path);
    }

    // Collect all .c files recursively
    let mut c_files = Vec::new();
    collect_c_files(area_path, area_path, &mut c_files)?;

    // Sort to get deterministic load order (daemons first, then alphabetical)
    c_files.sort_by(|a, b| {
        let a_is_daemon = a.starts_with("daemons/");
        let b_is_daemon = b.starts_with("daemons/");
        match (a_is_daemon, b_is_daemon) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.cmp(b),
        }
    });

    let mut loaded_objects = Vec::new();
    let area_key = area_id.key();

    for relative_path in &c_files {
        let full_path = area_path.join(relative_path);
        let source = std::fs::read_to_string(&full_path)
            .map_err(|e| anyhow::anyhow!("reading {}: {}", full_path.display(), e))?;

        // The object path in the VM is area_key/relative_path (without .c extension)
        let object_path = format!("/{}/{}", area_key, relative_path.trim_end_matches(".c"));

        match state.vm.compile_and_load(&object_path, &source) {
            Ok(obj_ref) => {
                tracing::debug!("compiled {} -> {}", relative_path, object_path);
                loaded_objects.push(object_path.clone());

                // Call create() if it exists
                if state.vm.has_function(&obj_ref, "create") {
                    match state.vm.call_function(&obj_ref, "create", &[]) {
                        Ok(_) => {
                            tracing::debug!("called create() on {}", object_path);
                        }
                        Err(e) => {
                            tracing::warn!("create() failed on {}: {}", object_path, e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("compile failed for {}: {}", relative_path, e);
                return Err(anyhow::anyhow!("compile error in {}: {}", relative_path, e));
            }
        }
    }

    tracing::info!(
        "loaded area {} with {} objects",
        area_key,
        loaded_objects.len()
    );
    Ok(loaded_objects)
}

/// Recursively collect all .c files relative to the root path.
fn collect_c_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> anyhow::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("reading directory {}: {}", dir.display(), e))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            collect_c_files(root, &path, out)?;
        } else if let Some(ext) = path.extension() {
            if ext == "c" {
                if let Ok(relative) = path.strip_prefix(root) {
                    out.push(relative.to_string_lossy().to_string());
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Area unloading
// ---------------------------------------------------------------------------

fn unload_area_internal(state: &mut AdapterState, area_key: &str) {
    if let Some(area) = state.areas.remove(area_key) {
        // Destruct all objects in this area from the VM
        for object_path in &area.loaded_objects {
            if let Some(obj_ref) = state.vm.object_table.find_object(object_path) {
                if let Err(e) = state.vm.object_table.destruct(&obj_ref) {
                    tracing::warn!("destruct failed for {}: {}", object_path, e);
                }
            }
        }
        tracing::info!("unloaded area {}", area_key);
    }
}

// ---------------------------------------------------------------------------
// Surgical program reload
// ---------------------------------------------------------------------------

fn reload_programs(
    state: &mut AdapterState,
    area_id: &AreaId,
    path: &str,
    files: &[String],
) -> Vec<AdapterMessage> {
    let area_key = area_id.key();
    let area_path = Path::new(path);
    let mut responses = Vec::new();

    for file in files {
        // Only handle .c files
        if !file.ends_with(".c") {
            continue;
        }

        let full_path = area_path.join(file);
        let object_path = format!("/{}/{}", area_key, file.trim_end_matches(".c"));

        let source = match std::fs::read_to_string(&full_path) {
            Ok(s) => s,
            Err(e) => {
                responses.push(AdapterMessage::ProgramReloadError {
                    area_id: area_id.clone(),
                    path: file.clone(),
                    error: format!("reading file: {}", e),
                });
                continue;
            }
        };

        // Try to compile the new version
        match state.vm.compile_and_load(&object_path, &source) {
            Ok(obj_ref) => {
                // Get the version from the master object
                let version = state
                    .vm
                    .object_table
                    .get_master(&object_path)
                    .map(|m| m.version)
                    .unwrap_or(1);

                // Call create() on the reloaded object
                if state.vm.has_function(&obj_ref, "create") {
                    if let Err(e) = state.vm.call_function(&obj_ref, "create", &[]) {
                        tracing::warn!("create() failed on reloaded {}: {}", object_path, e);
                    }
                }

                // Update loaded_objects list for the area
                if let Some(area_state) = state.areas.get_mut(&area_key) {
                    if !area_state.loaded_objects.contains(&object_path) {
                        area_state.loaded_objects.push(object_path.clone());
                    }
                }

                responses.push(AdapterMessage::ProgramReloaded {
                    area_id: area_id.clone(),
                    path: file.clone(),
                    version,
                });
                tracing::info!("reloaded {} (v{})", object_path, version);
            }
            Err(e) => {
                responses.push(AdapterMessage::ProgramReloadError {
                    area_id: area_id.clone(),
                    path: file.clone(),
                    error: format!("compile error: {}", e),
                });
            }
        }
    }

    responses
}

// ---------------------------------------------------------------------------
// Session handling
// ---------------------------------------------------------------------------

fn handle_session_start(
    state: &mut AdapterState,
    session_id: SessionId,
    username: String,
) -> Vec<AdapterMessage> {
    state.sessions.insert(
        session_id,
        SessionState {
            session_id,
            username: username.clone(),
            area_key: None,
        },
    );

    let mut responses = Vec::new();

    // Find the first loaded area and place the player there
    if let Some(area_key) = state.areas.keys().next().cloned() {
        if let Some(session) = state.sessions.get_mut(&session_id) {
            session.area_key = Some(area_key.clone());
        }

        // Try to call a connect function in the area daemon
        let daemon_path = format!("/{}/daemons/area_daemon", area_key);
        if let Some(obj_ref) = state.vm.object_table.find_object(&daemon_path) {
            if state.vm.has_function(&obj_ref, "on_connect") {
                let args = [
                    LpcValue::Int(session_id as i64),
                    LpcValue::String(username.clone()),
                ];
                match state.vm.call_function(&obj_ref, "on_connect", &args) {
                    Ok(result) => {
                        if let LpcValue::String(text) = result {
                            responses.push(AdapterMessage::SessionOutput { session_id, text });
                        }
                    }
                    Err(e) => {
                        tracing::warn!("on_connect failed: {}", e);
                    }
                }
            }
        }
    }

    // Send a welcome message
    responses.push(AdapterMessage::SessionOutput {
        session_id,
        text: format!("Welcome, {}! [LPC adapter]\n> ", username),
    });

    responses
}

fn handle_session_input(
    state: &mut AdapterState,
    session_id: SessionId,
    line: &str,
) -> Vec<AdapterMessage> {
    let session = match state.sessions.get(&session_id) {
        Some(s) => s,
        None => {
            tracing::warn!("input for unknown session {}", session_id);
            return vec![];
        }
    };

    let area_key = match &session.area_key {
        Some(k) => k.clone(),
        None => {
            return vec![AdapterMessage::SessionOutput {
                session_id,
                text: "You are not in any area.\n> ".to_string(),
            }];
        }
    };

    // Try to route the command to the area daemon's process_command function
    let daemon_path = format!("/{}/daemons/area_daemon", area_key);
    if let Some(obj_ref) = state.vm.object_table.find_object(&daemon_path) {
        if state.vm.has_function(&obj_ref, "process_command") {
            let args = [
                LpcValue::Int(session_id as i64),
                LpcValue::String(line.to_string()),
            ];
            match state.vm.call_function(&obj_ref, "process_command", &args) {
                Ok(result) => {
                    let text = match result {
                        LpcValue::String(s) => s,
                        LpcValue::Nil => "Done.\n".to_string(),
                        other => format!("{}\n", lpc_value_to_string(&other)),
                    };
                    return vec![AdapterMessage::SessionOutput {
                        session_id,
                        text: format!("{}\n> ", text.trim_end()),
                    }];
                }
                Err(e) => {
                    return vec![AdapterMessage::SessionOutput {
                        session_id,
                        text: format!("Error: {}\n> ", e),
                    }];
                }
            }
        }
    }

    // Fallback: echo the command
    vec![AdapterMessage::SessionOutput {
        session_id,
        text: format!("Unknown command: {}\n> ", line.trim()),
    }]
}

fn handle_session_end(state: &mut AdapterState, session_id: SessionId) -> Vec<AdapterMessage> {
    if let Some(session) = state.sessions.remove(&session_id) {
        // Try to call on_disconnect on the area daemon
        if let Some(area_key) = &session.area_key {
            let daemon_path = format!("/{}/daemons/area_daemon", area_key);
            if let Some(obj_ref) = state.vm.object_table.find_object(&daemon_path) {
                if state.vm.has_function(&obj_ref, "on_disconnect") {
                    let args = [
                        LpcValue::Int(session_id as i64),
                        LpcValue::String(session.username.clone()),
                    ];
                    if let Err(e) = state.vm.call_function(&obj_ref, "on_disconnect", &args) {
                        tracing::warn!("on_disconnect failed: {}", e);
                    }
                }
            }
        }
        tracing::info!("session {} ({}) ended", session_id, session.username);
    }
    vec![]
}

// ---------------------------------------------------------------------------
// Object call handling
// ---------------------------------------------------------------------------

fn handle_call(
    state: &mut AdapterState,
    request_id: u64,
    _object_id: u64,
    method: &str,
    args: &[Value],
) -> AdapterMessage {
    // Try to find the object. The object_id is opaque to us; for now, we
    // interpret it as 0 meaning "adapter-level call" and try to dispatch
    // to a well-known daemon.

    // Convert MOP Value args to LpcValue args
    let lpc_args: Vec<LpcValue> = args.iter().map(mop_value_to_lpc).collect();

    // Look for the method on any loaded daemon
    for area_state in state.areas.values() {
        for object_path in &area_state.loaded_objects {
            if let Some(obj_ref) = state.vm.object_table.find_object(object_path) {
                if state.vm.has_function(&obj_ref, method) {
                    match state.vm.call_function(&obj_ref, method, &lpc_args) {
                        Ok(result) => {
                            return AdapterMessage::CallResult {
                                request_id,
                                result: lpc_value_to_mop(&result),
                                cache: None,
                            };
                        }
                        Err(e) => {
                            return AdapterMessage::CallError {
                                request_id,
                                error: format!("{}", e),
                            };
                        }
                    }
                }
            }
        }
    }

    AdapterMessage::CallError {
        request_id,
        error: format!("method '{}' not found on any loaded object", method),
    }
}

// ---------------------------------------------------------------------------
// Web data
// ---------------------------------------------------------------------------

fn handle_get_web_data(
    state: &mut AdapterState,
    request_id: u64,
    area_key: &str,
) -> AdapterMessage {
    // Try calling query_web_data on the area daemon
    let daemon_path = format!("/{}/daemons/area_daemon", area_key);
    if let Some(obj_ref) = state.vm.object_table.find_object(&daemon_path) {
        if state.vm.has_function(&obj_ref, "query_web_data") {
            match state.vm.call_function(&obj_ref, "query_web_data", &[]) {
                Ok(result) => {
                    return AdapterMessage::CallResult {
                        request_id,
                        result: lpc_value_to_mop(&result),
                        cache: None,
                    };
                }
                Err(e) => {
                    tracing::warn!("query_web_data failed for {}: {}", area_key, e);
                }
            }
        }
    }

    // Return empty map on failure
    AdapterMessage::CallResult {
        request_id,
        result: Value::Map(HashMap::new()),
        cache: None,
    }
}

// ---------------------------------------------------------------------------
// Value conversion helpers
// ---------------------------------------------------------------------------

fn mop_value_to_lpc(value: &Value) -> LpcValue {
    match value {
        Value::Null => LpcValue::Nil,
        Value::Bool(b) => LpcValue::Int(if *b { 1 } else { 0 }),
        Value::Int(i) => LpcValue::Int(*i),
        Value::Float(f) => LpcValue::Float(*f),
        Value::String(s) => LpcValue::String(s.clone()),
        Value::Array(arr) => LpcValue::Array(arr.iter().map(mop_value_to_lpc).collect()),
        Value::Map(map) => {
            let pairs = map
                .iter()
                .map(|(k, v)| (LpcValue::String(k.clone()), mop_value_to_lpc(v)))
                .collect();
            LpcValue::Mapping(pairs)
        }
    }
}

fn lpc_value_to_mop(value: &LpcValue) -> Value {
    match value {
        LpcValue::Nil => Value::Null,
        LpcValue::Int(i) => Value::Int(*i),
        LpcValue::Float(f) => Value::Float(*f),
        LpcValue::String(s) => Value::String(s.clone()),
        LpcValue::Array(arr) => Value::Array(arr.iter().map(lpc_value_to_mop).collect()),
        LpcValue::Mapping(pairs) => {
            let map: HashMap<String, Value> = pairs
                .iter()
                .filter_map(|(k, v)| {
                    if let LpcValue::String(key) = k {
                        Some((key.clone(), lpc_value_to_mop(v)))
                    } else {
                        // Non-string keys get stringified
                        Some((lpc_value_to_string(k), lpc_value_to_mop(v)))
                    }
                })
                .collect();
            Value::Map(map)
        }
        LpcValue::Object(obj_ref) => {
            Value::String(format!("object:{}#{}", obj_ref.path, obj_ref.id))
        }
    }
}

fn lpc_value_to_string(value: &LpcValue) -> String {
    match value {
        LpcValue::Nil => "nil".to_string(),
        LpcValue::Int(i) => i.to_string(),
        LpcValue::Float(f) => f.to_string(),
        LpcValue::String(s) => s.clone(),
        LpcValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(lpc_value_to_string).collect();
            format!("({{ {} }})", items.join(", "))
        }
        LpcValue::Mapping(pairs) => {
            let items: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", lpc_value_to_string(k), lpc_value_to_string(v)))
                .collect();
            format!("([ {} ])", items.join(", "))
        }
        LpcValue::Object(obj_ref) => format!("object:{}#{}", obj_ref.path, obj_ref.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_builder_access_returns_call_result() {
        let mut state = AdapterState {
            areas: HashMap::new(),
            sessions: HashMap::new(),
            vm: Vm::new(),
            stdlib_db_url: None,
        };

        let responses = handle_message(
            &mut state,
            DriverMessage::CheckBuilderAccess {
                request_id: 42,
                user: "alice".into(),
                namespace: "ns".into(),
                area: "area".into(),
                action: "read".into(),
            },
        )
        .await;

        assert_eq!(
            responses,
            vec![AdapterMessage::CallResult {
                request_id: 42,
                result: Value::Map(HashMap::from([("allowed".into(), Value::Bool(true),)])),
                cache: None,
            }]
        );
    }
}
