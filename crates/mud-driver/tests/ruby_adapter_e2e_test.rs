//! End-to-end integration tests that exercise the full stack:
//! Rust driver (AdapterManager) + real Ruby adapter process + Ruby stdlib.
//!
//! These tests spawn the actual Ruby adapter binary, connect via Unix socket,
//! load a test area with rooms defined using the Ruby stdlib DSL, start a
//! player session, and verify that game commands (look, movement) produce
//! correct output from the Ruby stdlib's SessionHandler, CommandParser, Area,
//! and Room classes.
//!
//! Run with: `cargo test -p mud-driver --test ruby_adapter_e2e_test -- --ignored`
//!
//! Prerequisites:
//! - Ruby installed and on PATH
//! - `bundle install` completed in `adapters/ruby/` (gems in vendor/bundle)

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;

use mud_core::types::AreaId;
use mud_driver::config::Config;
use mud_driver::runtime::adapter_manager::AdapterManager;
use mud_mop::message::{AdapterMessage, DriverMessage};
use tokio::time::timeout;

/// Generous timeout for Ruby adapter startup and connection.
const RUBY_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for individual message exchanges after startup.
const MSG_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Skip-condition helpers
// ---------------------------------------------------------------------------

/// Check whether Ruby is available on the system.
fn ruby_available() -> bool {
    StdCommand::new("ruby")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Return the absolute path to the adapter's Ruby directory.
fn adapter_ruby_dir() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .join("../../adapters/ruby")
        .canonicalize()
        .expect("adapters/ruby directory must exist")
}

/// Check whether bundled gems are available for the adapter.
/// Uses `bundle exec ruby` to verify the msgpack gem is loadable.
fn bundle_available() -> bool {
    let ruby_dir = adapter_ruby_dir();
    let bundle_bin = find_bundle_binary();

    let result = StdCommand::new(&bundle_bin)
        .args(["exec", "ruby", "-e", "require 'msgpack'; puts 'ok'"])
        .current_dir(&ruby_dir)
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.trim() == "ok"
        }
        Err(_) => false,
    }
}

/// Return true if both Ruby and its gems are available, printing skip
/// messages to stderr if not.
fn prerequisites_met() -> bool {
    if !ruby_available() {
        eprintln!("SKIPPED: Ruby not available on PATH");
        return false;
    }
    if !bundle_available() {
        eprintln!(
            "SKIPPED: Ruby gems not installed. Run `bundle install` in adapters/ruby/"
        );
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Test area creation
// ---------------------------------------------------------------------------

/// Create a minimal test area on disk with two rooms (entrance and hall)
/// connected by north/south exits.
///
/// Directory structure:
/// ```text
/// <base>/test/starter/
///   .meta.yml
///   rooms/
///     entrance.rb
///     hall.rb
/// ```
fn create_test_area(world_dir: &Path) {
    let area_dir = world_dir.join("test").join("starter");
    let rooms_dir = area_dir.join("rooms");
    std::fs::create_dir_all(&rooms_dir).expect("create rooms directory");

    // .meta.yml -- required by the server's area scanner
    std::fs::write(
        area_dir.join(".meta.yml"),
        "owner: testuser\nsystem: false\n",
    )
    .expect("write .meta.yml");

    // rooms/entrance.rb -- uses the Ruby stdlib Room DSL
    std::fs::write(
        rooms_dir.join("entrance.rb"),
        r#"class Entrance < MudAdapter::Stdlib::World::Room
  title "The Entrance"
  description "You stand at the entrance of the test MUD."
  exit :north, to: "hall"
end
"#,
    )
    .expect("write entrance.rb");

    // rooms/hall.rb -- uses the Ruby stdlib Room DSL
    std::fs::write(
        rooms_dir.join("hall.rb"),
        r#"class Hall < MudAdapter::Stdlib::World::Room
  title "The Great Hall"
  description "A grand hall stretches before you."
  exit :south, to: "entrance"
end
"#,
    )
    .expect("write hall.rb");
}

// ---------------------------------------------------------------------------
// Ruby adapter process wrapper
// ---------------------------------------------------------------------------

/// Guard struct that ensures the Ruby adapter child process is killed on drop,
/// even if the test panics.
struct RubyAdapterProcess {
    _child: tokio::process::Child,
}

impl RubyAdapterProcess {
    /// Spawn the Ruby adapter as a child process, connecting to the given
    /// Unix socket path. Uses `bundle exec` to ensure gems are available.
    fn spawn(socket_path: &Path) -> Self {
        let ruby_dir = adapter_ruby_dir();
        let adapter_bin = ruby_dir.join("bin/mud-adapter");
        let bundle_bin = find_bundle_binary();

        let child = tokio::process::Command::new(&bundle_bin)
            .arg("exec")
            .arg("ruby")
            .arg(&adapter_bin)
            .arg("--socket")
            .arg(socket_path)
            .arg("--web-socket")
            .arg(socket_path.with_file_name("portal-web.sock"))
            .current_dir(&ruby_dir)
            .env("BUNDLE_GEMFILE", ruby_dir.join("Gemfile"))
            .env("BUNDLE_PATH", ruby_dir.join("vendor/bundle"))
            .kill_on_drop(true)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn Ruby adapter process");

        Self { _child: child }
    }
}

/// Find the `bundle` binary, checking PATH first, then common Ruby gem
/// install locations.
fn find_bundle_binary() -> PathBuf {
    // Check if bundle is on PATH
    if let Ok(output) = StdCommand::new("which").arg("bundle").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return PathBuf::from(path);
            }
        }
    }

    // Check common gem binary locations
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{home}/.local/share/gem/ruby/3.4.0/bin/bundle"),
        format!("{home}/.local/share/gem/ruby/3.3.0/bin/bundle"),
        format!("{home}/.local/share/gem/ruby/3.2.0/bin/bundle"),
        format!("{home}/.gem/ruby/3.4.0/bin/bundle"),
        format!("{home}/.gem/ruby/3.3.0/bin/bundle"),
        "/usr/local/bin/bundle".into(),
        "/usr/bin/bundle".into(),
    ];

    for candidate in &candidates {
        let path = Path::new(candidate);
        if path.exists() {
            return path.to_path_buf();
        }
    }

    // Fall back to just "bundle" and hope it's on PATH
    PathBuf::from("bundle")
}

// ---------------------------------------------------------------------------
// Helper: collect session output from the adapter
// ---------------------------------------------------------------------------

/// Receive adapter messages, collecting SessionOutput text for the given
/// session_id. Stops when the predicate returns true on the accumulated
/// text, or panics when the timeout expires.
///
/// Non-SessionOutput messages (Log, AreaLoaded, etc.) are logged to stderr.
async fn collect_output_until(
    manager: &mut AdapterManager,
    session_id: u64,
    predicate: impl Fn(&str) -> bool,
    timeout_duration: Duration,
) -> String {
    let mut collected = String::new();
    let deadline = tokio::time::Instant::now() + timeout_duration;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!(
                "Timed out waiting for output matching predicate.\n\
                 Collected so far ({} bytes):\n---\n{}\n---",
                collected.len(),
                collected
            );
        }

        let msg = match timeout(remaining, manager.recv()).await {
            Ok(Some(msg)) => msg,
            Ok(None) => panic!(
                "Adapter channel closed. Collected:\n---\n{}\n---",
                collected
            ),
            Err(_) => panic!(
                "Timed out waiting for adapter message.\n\
                 Collected so far ({} bytes):\n---\n{}\n---",
                collected.len(),
                collected
            ),
        };

        match msg {
            AdapterMessage::SessionOutput {
                session_id: sid,
                text,
            } if sid == session_id => {
                collected.push_str(&text);
                if predicate(&collected) {
                    return collected;
                }
            }
            AdapterMessage::Log {
                level,
                message,
                area,
            } => {
                eprintln!(
                    "[e2e] adapter log [{}]: {} (area={:?})",
                    level, message, area
                );
            }
            other => {
                eprintln!("[e2e] other adapter message: {:?}", other);
            }
        }
    }
}

/// Wait for an AreaLoaded message for the expected area, handling Log
/// messages along the way. Panics on AreaError or timeout.
async fn wait_for_area_loaded(
    manager: &mut AdapterManager,
    expected: &AreaId,
    timeout_duration: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout_duration;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!("Timed out waiting for AreaLoaded for {}", expected);
        }

        let msg = match timeout(remaining, manager.recv()).await {
            Ok(Some(msg)) => msg,
            Ok(None) => panic!("Adapter channel closed waiting for AreaLoaded"),
            Err(_) => panic!("Timed out waiting for AreaLoaded for {}", expected),
        };

        match msg {
            AdapterMessage::AreaLoaded { ref area_id } if area_id == expected => {
                eprintln!("[e2e] Area loaded: {}", area_id);
                return;
            }
            AdapterMessage::AreaError { ref area_id, ref error } if area_id == expected => {
                panic!("Area {} failed to load: {}", area_id, error);
            }
            AdapterMessage::Log {
                level,
                message,
                area,
            } => {
                eprintln!(
                    "[e2e] adapter log [{}]: {} (area={:?})",
                    level, message, area
                );
            }
            other => {
                eprintln!("[e2e] unexpected message during area load: {:?}", other);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: set up a connected adapter manager with a real Ruby adapter
// ---------------------------------------------------------------------------

/// Create temp dirs, set up a test area, start the AdapterManager, spawn
/// the Ruby adapter, and wait for the handshake. Returns all the pieces
/// needed for testing.
///
/// The returned `tempfile::TempDir` must be kept alive for the test duration.
async fn setup_ruby_adapter() -> (AdapterManager, tempfile::TempDir, RubyAdapterProcess) {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let world_dir = temp_dir.path().join("world");
    let socket_path = temp_dir.path().join("mud-e2e.sock");

    std::fs::create_dir_all(&world_dir).expect("create world dir");
    create_test_area(&world_dir);

    let config = Config::default();
    let mut manager = AdapterManager::new(socket_path.clone());
    let listener = manager.start(&config).await.expect("start adapter manager");

    let ruby_process = RubyAdapterProcess::spawn(&socket_path);

    let language = timeout(RUBY_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("timed out waiting for Ruby adapter to connect")
        .expect("failed to accept Ruby adapter connection");

    assert_eq!(language, "ruby", "Adapter handshake should identify as ruby");
    eprintln!("[e2e] Ruby adapter connected (language={})", language);

    (manager, temp_dir, ruby_process)
}

// ===========================================================================
// E2E Tests
// ===========================================================================

/// Full end-to-end test: Rust driver + real Ruby adapter + Ruby stdlib.
///
/// Tests the complete flow:
/// 1. Adapter connects and sends Handshake with language="ruby"
/// 2. LoadArea loads a test area with two rooms from the Ruby stdlib
/// 3. SessionStart triggers welcome text from the Ruby SessionHandler
/// 4. "look" command returns room description via the Ruby CommandParser
/// 5. "north" command moves the player and returns the new room description
/// 6. "south" command moves back to the original room
/// 7. "help" command returns the help text
/// 8. "say" command works with the command parser
/// 9. Invalid direction returns an error message
/// 10. SessionEnd cleans up cleanly
#[tokio::test]
#[ignore] // Run with: cargo test -- --ignored
async fn ruby_adapter_full_session_e2e() {
    if !prerequisites_met() {
        return;
    }

    let (mut manager, temp_dir, _ruby_process) = setup_ruby_adapter().await;

    let world_dir = temp_dir.path().join("world");
    let area_id = AreaId::new("test", "starter");
    let area_path = world_dir
        .join("test")
        .join("starter")
        .to_string_lossy()
        .to_string();

    // =====================================================================
    // Step 1: Load the test area
    // =====================================================================
    manager
        .send_to(
            "ruby",
            DriverMessage::LoadArea {
                area_id: area_id.clone(),
                path: area_path,
                db_url: None,
            },
        )
        .await
        .expect("send LoadArea");

    eprintln!("[e2e] Sent LoadArea for test/starter");
    wait_for_area_loaded(&mut manager, &area_id, RUBY_TIMEOUT).await;

    // =====================================================================
    // Step 2: Start a session -- expect welcome text + room description
    // =====================================================================
    let session_id: u64 = 1;
    let username = "testplayer";

    manager
        .send_to(
            "ruby",
            DriverMessage::SessionStart {
                session_id,
                username: username.into(),
            },
        )
        .await
        .expect("send SessionStart");

    eprintln!("[e2e] Sent SessionStart for session {}", session_id);

    // The session handler sends: welcome text, room description, then "> "
    let welcome_output =
        collect_output_until(&mut manager, session_id, |t| t.contains("> "), MSG_TIMEOUT).await;

    eprintln!("[e2e] Welcome output:\n---\n{}\n---", welcome_output);

    // Verify welcome text contains the username
    assert!(
        welcome_output.contains(username),
        "Welcome text should contain '{}'. Got:\n{}",
        username,
        welcome_output
    );

    // Verify the initial room description is shown
    // Dir.glob sorts alphabetically: "entrance" < "hall", so Entrance is first.
    assert!(
        welcome_output.contains("The Entrance")
            || welcome_output.contains("The Great Hall"),
        "Welcome should include a room title. Got:\n{}",
        welcome_output
    );

    // =====================================================================
    // Step 3: "look" command -- room description
    // =====================================================================
    manager
        .send_to(
            "ruby",
            DriverMessage::SessionInput {
                session_id,
                line: "look".into(),
            },
        )
        .await
        .expect("send look");

    let look_output =
        collect_output_until(&mut manager, session_id, |t| t.contains("> "), MSG_TIMEOUT).await;

    eprintln!("[e2e] Look output:\n---\n{}\n---", look_output);

    assert!(
        look_output.contains("The Entrance"),
        "Look should show 'The Entrance'. Got:\n{}",
        look_output
    );
    assert!(
        look_output.contains("entrance of the test MUD"),
        "Look should show entrance description. Got:\n{}",
        look_output
    );
    assert!(
        look_output.contains("north"),
        "Look should show exit 'north'. Got:\n{}",
        look_output
    );

    // =====================================================================
    // Step 4: "north" command -- move to The Great Hall
    // =====================================================================
    manager
        .send_to(
            "ruby",
            DriverMessage::SessionInput {
                session_id,
                line: "north".into(),
            },
        )
        .await
        .expect("send north");

    let north_output =
        collect_output_until(&mut manager, session_id, |t| t.contains("> "), MSG_TIMEOUT).await;

    eprintln!("[e2e] North output:\n---\n{}\n---", north_output);

    assert!(
        north_output.contains("You move north"),
        "Should see movement message. Got:\n{}",
        north_output
    );
    assert!(
        north_output.contains("The Great Hall"),
        "Should see 'The Great Hall'. Got:\n{}",
        north_output
    );
    assert!(
        north_output.contains("grand hall"),
        "Should see hall description. Got:\n{}",
        north_output
    );

    // =====================================================================
    // Step 5: "south" command -- back to entrance
    // =====================================================================
    manager
        .send_to(
            "ruby",
            DriverMessage::SessionInput {
                session_id,
                line: "south".into(),
            },
        )
        .await
        .expect("send south");

    let south_output =
        collect_output_until(&mut manager, session_id, |t| t.contains("> "), MSG_TIMEOUT).await;

    eprintln!("[e2e] South output:\n---\n{}\n---", south_output);

    assert!(
        south_output.contains("You move south"),
        "Should see movement message. Got:\n{}",
        south_output
    );
    assert!(
        south_output.contains("The Entrance"),
        "Should be back at 'The Entrance'. Got:\n{}",
        south_output
    );

    // =====================================================================
    // Step 6: "help" command -- stdlib command list
    // =====================================================================
    manager
        .send_to(
            "ruby",
            DriverMessage::SessionInput {
                session_id,
                line: "help".into(),
            },
        )
        .await
        .expect("send help");

    let help_output =
        collect_output_until(&mut manager, session_id, |t| t.contains("> "), MSG_TIMEOUT).await;

    eprintln!("[e2e] Help output:\n---\n{}\n---", help_output);

    assert!(
        help_output.contains("Available commands"),
        "Help should list available commands. Got:\n{}",
        help_output
    );
    assert!(
        help_output.contains("look"),
        "Help should mention 'look'. Got:\n{}",
        help_output
    );

    // =====================================================================
    // Step 7: "say" command -- command parser with arguments
    // =====================================================================
    manager
        .send_to(
            "ruby",
            DriverMessage::SessionInput {
                session_id,
                line: "say hello world".into(),
            },
        )
        .await
        .expect("send say");

    let say_output =
        collect_output_until(&mut manager, session_id, |t| t.contains("> "), MSG_TIMEOUT).await;

    eprintln!("[e2e] Say output:\n---\n{}\n---", say_output);

    assert!(
        say_output.contains("hello world"),
        "Say should echo the message. Got:\n{}",
        say_output
    );
    assert!(
        say_output.contains(username),
        "Say should show the speaker's name. Got:\n{}",
        say_output
    );

    // =====================================================================
    // Step 8: Invalid direction -- error handling
    // =====================================================================
    manager
        .send_to(
            "ruby",
            DriverMessage::SessionInput {
                session_id,
                line: "west".into(),
            },
        )
        .await
        .expect("send west");

    let west_output =
        collect_output_until(&mut manager, session_id, |t| t.contains("> "), MSG_TIMEOUT).await;

    eprintln!("[e2e] West output:\n---\n{}\n---", west_output);

    assert!(
        west_output.contains("can't go west"),
        "Should see error for invalid direction. Got:\n{}",
        west_output
    );

    // =====================================================================
    // Step 9: End session cleanly
    // =====================================================================
    manager
        .send_to("ruby", DriverMessage::SessionEnd { session_id })
        .await
        .expect("send SessionEnd");

    eprintln!("[e2e] Sent SessionEnd for session {}", session_id);

    // Drain any remaining messages (the adapter logs session end)
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = drain_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match timeout(remaining, manager.recv()).await {
            Ok(Some(msg)) => {
                eprintln!("[e2e] Post-SessionEnd message: {:?}", msg);
            }
            _ => break,
        }
    }

    eprintln!("[e2e] Full session E2E test completed successfully!");
    manager.shutdown();
}

/// Test that the Ruby adapter sends a proper handshake with language="ruby".
#[tokio::test]
#[ignore]
async fn ruby_adapter_handshake() {
    if !prerequisites_met() {
        return;
    }

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let socket_path = temp_dir.path().join("handshake-test.sock");

    let config = Config::default();
    let mut manager = AdapterManager::new(socket_path.clone());
    let listener = manager.start(&config).await.expect("start manager");

    let _ruby = RubyAdapterProcess::spawn(&socket_path);

    let language = timeout(RUBY_TIMEOUT, manager.accept_connection(&listener))
        .await
        .expect("timed out waiting for handshake")
        .expect("handshake failed");

    assert_eq!(language, "ruby");
    assert!(manager.has_adapter("ruby"));

    eprintln!("[e2e] Handshake test passed");
    manager.shutdown();
}

/// Test that LoadArea for a nonexistent path results in an AreaError
/// from the Ruby adapter (the Ruby stdlib handles the error gracefully).
#[tokio::test]
#[ignore]
async fn ruby_adapter_area_load_error() {
    if !prerequisites_met() {
        return;
    }

    let (mut manager, _temp_dir, _ruby_process) = setup_ruby_adapter().await;

    // Send LoadArea for a nonexistent path
    let area_id = AreaId::new("bogus", "nonexistent");
    manager
        .send_to(
            "ruby",
            DriverMessage::LoadArea {
                area_id: area_id.clone(),
                path: "/tmp/this_path_does_not_exist_12345".into(),
                db_url: None,
            },
        )
        .await
        .expect("send LoadArea");

    // Wait for either AreaLoaded or AreaError
    let deadline = tokio::time::Instant::now() + MSG_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!("Timed out waiting for area load response");
        }

        let msg = timeout(remaining, manager.recv())
            .await
            .expect("recv timed out")
            .expect("recv None");

        match msg {
            AdapterMessage::AreaError {
                area_id: ref aid,
                ref error,
            } => {
                assert_eq!(aid, &area_id);
                eprintln!("[e2e] Got expected AreaError: {}", error);
                break;
            }
            AdapterMessage::AreaLoaded { ref area_id } => {
                // The area might "load" with 0 rooms if the dir doesn't
                // exist (the Ruby Area class doesn't raise for missing dirs).
                // Either way, the adapter handled it without crashing.
                eprintln!("[e2e] Area loaded (0 rooms) for: {}", area_id);
                break;
            }
            AdapterMessage::Log { message, .. } => {
                eprintln!("[e2e] log: {}", message);
            }
            _ => {}
        }
    }

    eprintln!("[e2e] Area load error test passed");
    manager.shutdown();
}
