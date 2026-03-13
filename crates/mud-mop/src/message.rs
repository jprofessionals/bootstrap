use std::collections::HashMap;

use mud_core::types::{AreaId, ObjectId, SessionId};
use serde::{Deserialize, Serialize};

/// Dynamic value type for MOP message payloads.
///
/// Serialises as an untagged enum so MessagePack consumers see native
/// types (nil, bool, integer, float, string, array, map) rather than
/// wrapper objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Map(HashMap<String, Value>),
}

/// Cache policy hint returned with CallResult responses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CachePolicy {
    /// Never cache — always call through to the adapter.
    #[serde(rename = "volatile")]
    Volatile,
    /// Safe to cache until explicitly invalidated.
    #[serde(rename = "cacheable")]
    Cacheable,
    /// Cache with a time-to-live in seconds.
    #[serde(rename = "ttl")]
    Ttl(u64),
}

/// Messages sent from the driver to a language adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DriverMessage {
    /// Ask the adapter to load an area from disk.
    #[serde(rename = "load_area")]
    LoadArea {
        area_id: AreaId,
        path: String,
        db_url: Option<String>,
    },

    /// Ask the adapter to reload a previously loaded area.
    #[serde(rename = "reload_area")]
    ReloadArea {
        area_id: AreaId,
        path: String,
        db_url: Option<String>,
    },

    /// Ask the adapter to unload an area and free its resources.
    #[serde(rename = "unload_area")]
    UnloadArea { area_id: AreaId },

    /// A new player session has started.
    #[serde(rename = "session_start")]
    SessionStart {
        session_id: SessionId,
        username: String,
    },

    /// A line of input from a player session.
    #[serde(rename = "session_input")]
    SessionInput { session_id: SessionId, line: String },

    /// A player session has ended.
    #[serde(rename = "session_end")]
    SessionEnd { session_id: SessionId },

    /// Remote procedure call into an in-game object.
    #[serde(rename = "call")]
    Call {
        request_id: u64,
        object_id: ObjectId,
        method: String,
        args: Vec<Value>,
    },

    /// Keepalive ping; adapter must reply with `Pong`.
    #[serde(rename = "ping")]
    Ping { seq: u64 },

    /// Response to a DriverRequest, sent back to the adapter.
    #[serde(rename = "request_response")]
    RequestResponse { request_id: u64, result: Value },

    /// Error response to a DriverRequest.
    #[serde(rename = "request_error")]
    RequestError { request_id: u64, error: String },

    /// Configuration sent to the adapter after database setup.
    /// Provides the stdlib database URL so the adapter can run its own
    /// migrations and connect directly for schema management.
    #[serde(rename = "configure")]
    Configure { stdlib_db_url: String },

    /// Ask the adapter to check whether a builder has access to an area.
    #[serde(rename = "check_builder_access")]
    CheckBuilderAccess {
        request_id: u64,
        user: String,
        namespace: String,
        area: String,
        action: String,
    },

    /// Ask the adapter/stdlib to evaluate repo access policy.
    #[serde(rename = "check_repo_access")]
    CheckRepoAccess {
        request_id: u64,
        username: String,
        namespace: String,
        name: String,
        level: String,
    },

    /// Ask the adapter to reload specific changed files within an area (surgical reload).
    #[serde(rename = "reload_program")]
    ReloadProgram {
        area_id: AreaId,
        path: String,
        files: Vec<String>,
    },

    /// Ask the adapter to reload the active stdlib runtime.
    #[serde(rename = "reload_stdlib")]
    ReloadStdlib { subsystem: String },

    /// Ask the adapter to provide web template data for an area.
    #[serde(rename = "get_web_data")]
    GetWebData { request_id: u64, area_key: String },
}

/// Messages sent from a language adapter back to the driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AdapterMessage {
    /// Area loaded successfully.
    #[serde(rename = "area_loaded")]
    AreaLoaded { area_id: AreaId },

    /// Area failed to load.
    #[serde(rename = "area_error")]
    AreaError { area_id: AreaId, error: String },

    /// Text output destined for a player session.
    #[serde(rename = "session_output")]
    SessionOutput { session_id: SessionId, text: String },

    /// Successful result from a `Call` request.
    #[serde(rename = "call_result")]
    CallResult {
        request_id: u64,
        result: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache: Option<CachePolicy>,
    },

    /// Error result from a `Call` request.
    #[serde(rename = "call_error")]
    CallError { request_id: u64, error: String },

    /// Request to move an object to a new container/room.
    #[serde(rename = "move_object")]
    MoveObject {
        object_id: ObjectId,
        destination_id: ObjectId,
    },

    /// Send text to a specific player session.
    #[serde(rename = "send_message")]
    SendMessage {
        target_session: SessionId,
        text: String,
    },

    /// Log message from area code.
    #[serde(rename = "log")]
    Log {
        level: String,
        message: String,
        area: Option<String>,
    },

    /// Keepalive pong in response to a `Ping`.
    #[serde(rename = "pong")]
    Pong { seq: u64 },

    /// Initial handshake identifying the adapter.
    ///
    /// The `languages` field allows an adapter to handle multiple language
    /// types (e.g. the LPC adapter handles both "lpc" and "rust" areas).
    /// When empty, only the primary `language` is registered.
    #[serde(rename = "handshake")]
    Handshake {
        adapter_name: String,
        language: String,
        version: String,
        #[serde(default)]
        languages: Vec<String>,
    },

    /// Generic request from adapter to driver for portal operations.
    /// The adapter sends a request_id, an action string, and a params map.
    /// The driver processes it and replies with RequestResponse or RequestError.
    #[serde(rename = "driver_request")]
    DriverRequest {
        request_id: u64,
        action: String,
        params: Value,
    },

    /// Confirm a program was successfully reloaded, with new version number.
    #[serde(rename = "program_reloaded")]
    ProgramReloaded {
        area_id: AreaId,
        path: String,
        version: u64,
    },

    /// Report a program reload failure; old version retained.
    #[serde(rename = "program_reload_error")]
    ProgramReloadError {
        area_id: AreaId,
        path: String,
        error: String,
    },

    /// Notify all adapters that cached values for these objects are stale.
    #[serde(rename = "invalidate_cache")]
    InvalidateCache { object_ids: Vec<ObjectId> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_message_round_trip() {
        let msg = DriverMessage::LoadArea {
            area_id: AreaId::new("system", "lobby"),
            path: "/world/system/lobby".into(),
            db_url: None,
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn check_repo_access_round_trip() {
        let msg = DriverMessage::CheckRepoAccess {
            request_id: 77,
            username: "alice".into(),
            namespace: "system".into(),
            name: "stdlib".into(),
            level: "read_write".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn reload_stdlib_round_trip() {
        let msg = DriverMessage::ReloadStdlib {
            subsystem: "portal".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn adapter_message_round_trip() {
        let msg = AdapterMessage::Handshake {
            adapter_name: "mud-adapter-ruby".into(),
            language: "ruby".into(),
            version: "0.1.0".into(),
            languages: vec![],
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn adapter_handshake_with_languages() {
        let msg = AdapterMessage::Handshake {
            adapter_name: "mud-adapter-lpc".into(),
            language: "lpc".into(),
            version: "0.1.0".into(),
            languages: vec!["lpc".into(), "rust".into()],
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
        match decoded {
            AdapterMessage::Handshake { languages, .. } => {
                assert_eq!(languages, vec!["lpc", "rust"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn adapter_handshake_without_languages_defaults_to_empty() {
        // Simulate a legacy adapter that omits the `languages` field entirely
        // from the wire format. Build a MessagePack map manually with only the
        // fields a pre-languages adapter would send.
        #[derive(Serialize)]
        #[serde(tag = "type")]
        enum LegacyAdapter {
            #[serde(rename = "handshake")]
            Handshake {
                adapter_name: String,
                language: String,
                version: String,
            },
        }

        let legacy = LegacyAdapter::Handshake {
            adapter_name: "mud-adapter-ruby".into(),
            language: "ruby".into(),
            version: "0.1.0".into(),
        };
        let bytes = rmp_serde::to_vec_named(&legacy).expect("serialize legacy");
        // Deserialize as the real AdapterMessage — languages should default to empty
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        match decoded {
            AdapterMessage::Handshake {
                languages,
                adapter_name,
                language,
                ..
            } => {
                assert!(
                    languages.is_empty(),
                    "languages should default to empty vec"
                );
                assert_eq!(adapter_name, "mud-adapter-ruby");
                assert_eq!(language, "ruby");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn value_round_trip() {
        let val = Value::Map(HashMap::from([
            ("name".into(), Value::String("sword".into())),
            ("damage".into(), Value::Int(42)),
            ("enchanted".into(), Value::Bool(true)),
            ("weight".into(), Value::Float(3.5)),
            ("lore".into(), Value::Null),
            (
                "tags".into(),
                Value::Array(vec![
                    Value::String("weapon".into()),
                    Value::String("melee".into()),
                ]),
            ),
        ]));
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn call_with_args_round_trip() {
        let msg = DriverMessage::Call {
            request_id: 1,
            object_id: 100,
            method: "take".into(),
            args: vec![Value::String("sword".into()), Value::Int(1)],
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn log_with_optional_area() {
        let with_area = AdapterMessage::Log {
            level: "info".into(),
            message: "Area loaded".into(),
            area: Some("system/lobby".into()),
        };
        let bytes = rmp_serde::to_vec_named(&with_area).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(with_area, decoded);

        let without_area = AdapterMessage::Log {
            level: "warn".into(),
            message: "Something happened".into(),
            area: None,
        };
        let bytes = rmp_serde::to_vec_named(&without_area).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(without_area, decoded);
    }

    // -- Additional DriverMessage variant round-trips --

    #[test]
    fn reload_area_round_trip() {
        let msg = DriverMessage::ReloadArea {
            area_id: AreaId::new("game", "tavern"),
            path: "/world/game/tavern".into(),
            db_url: None,
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn unload_area_round_trip() {
        let msg = DriverMessage::UnloadArea {
            area_id: AreaId::new("game", "dungeon"),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn session_start_round_trip() {
        let msg = DriverMessage::SessionStart {
            session_id: 42,
            username: "alice".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn session_input_round_trip() {
        let msg = DriverMessage::SessionInput {
            session_id: 42,
            line: "look".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn session_end_round_trip() {
        let msg = DriverMessage::SessionEnd { session_id: 42 };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn ping_round_trip() {
        let msg = DriverMessage::Ping { seq: 100 };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn request_response_round_trip() {
        let msg = DriverMessage::RequestResponse {
            request_id: 7,
            result: Value::Map(HashMap::from([
                ("status".into(), Value::String("ok".into())),
                ("count".into(), Value::Int(3)),
            ])),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn configure_round_trip() {
        let msg = DriverMessage::Configure {
            stdlib_db_url: "postgres://user:pass@localhost:5432/mud_stdlib".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn request_error_round_trip() {
        let msg = DriverMessage::RequestError {
            request_id: 8,
            error: "not found".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    // -- Additional AdapterMessage variant round-trips --

    #[test]
    fn area_loaded_round_trip() {
        let msg = AdapterMessage::AreaLoaded {
            area_id: AreaId::new("system", "lobby"),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn area_error_round_trip() {
        let msg = AdapterMessage::AreaError {
            area_id: AreaId::new("game", "broken"),
            error: "syntax error line 42".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn session_output_round_trip() {
        let msg = AdapterMessage::SessionOutput {
            session_id: 1,
            text: "You are in a dark room.\n> ".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn call_result_round_trip() {
        let msg = AdapterMessage::CallResult {
            request_id: 5,
            result: Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            cache: None,
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn call_error_round_trip() {
        let msg = AdapterMessage::CallError {
            request_id: 6,
            error: "method not found".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn move_object_round_trip() {
        let msg = AdapterMessage::MoveObject {
            object_id: 100,
            destination_id: 200,
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn send_message_round_trip() {
        let msg = AdapterMessage::SendMessage {
            target_session: 42,
            text: "Hello player!".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn pong_round_trip() {
        let msg = AdapterMessage::Pong { seq: 99 };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn driver_request_round_trip() {
        let msg = AdapterMessage::DriverRequest {
            request_id: 11,
            action: "list_players".into(),
            params: Value::Map(HashMap::from([("limit".into(), Value::Int(10))])),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn load_area_with_db_url_round_trip() {
        let msg = DriverMessage::LoadArea {
            area_id: AreaId::new("game", "tavern"),
            path: "/world/game/tavern".into(),
            db_url: Some(
                "postgres://mud_area_game_tavern:secret@localhost:5432/mud_area_game_tavern".into(),
            ),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn reload_area_with_db_url_round_trip() {
        let msg = DriverMessage::ReloadArea {
            area_id: AreaId::new("game", "tavern"),
            path: "/world/game/tavern".into(),
            db_url: Some("postgres://user:pass@localhost/db".into()),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);
    }

    // -- Value type tests --

    #[test]
    fn value_null() {
        let val = Value::Null;
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_bool() {
        let val = Value::Bool(true);
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_int() {
        let val = Value::Int(-42);
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_float() {
        let val = Value::Float(3.14);
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_string_empty() {
        let val = Value::String(String::new());
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_array_empty() {
        let val = Value::Array(vec![]);
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_map_empty() {
        let val = Value::Map(HashMap::new());
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_nested_array() {
        let val = Value::Array(vec![
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Array(vec![Value::Int(3)]),
        ]);
        let bytes = rmp_serde::to_vec_named(&val).expect("serialize");
        let decoded: Value = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(val, decoded);
    }

    #[test]
    fn value_clone() {
        let val = Value::Map(HashMap::from([(
            "key".into(),
            Value::String("value".into()),
        )]));
        let cloned = val.clone();
        assert_eq!(val, cloned);
    }

    #[test]
    fn value_debug() {
        let val = Value::String("hello".into());
        let debug = format!("{:?}", val);
        assert!(debug.contains("hello"));
    }

    #[test]
    fn test_check_builder_access_serialization() {
        let msg = DriverMessage::CheckBuilderAccess {
            request_id: 42,
            user: "alice".into(),
            namespace: "game".into(),
            area: "tavern".into(),
            action: "write".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);

        // Verify all fields survived the round-trip
        if let DriverMessage::CheckBuilderAccess {
            request_id,
            user,
            namespace,
            area,
            action,
        } = &decoded
        {
            assert_eq!(*request_id, 42);
            assert_eq!(user, "alice");
            assert_eq!(namespace, "game");
            assert_eq!(area, "tavern");
            assert_eq!(action, "write");
        } else {
            panic!("decoded to wrong variant");
        }
    }

    #[test]
    fn test_get_web_data_serialization() {
        let msg = DriverMessage::GetWebData {
            request_id: 99,
            area_key: "game/tavern".into(),
        };
        let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
        let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(msg, decoded);

        // Verify all fields survived the round-trip
        if let DriverMessage::GetWebData {
            request_id,
            area_key,
        } = &decoded
        {
            assert_eq!(*request_id, 99);
            assert_eq!(area_key, "game/tavern");
        } else {
            panic!("decoded to wrong variant");
        }
    }
}
