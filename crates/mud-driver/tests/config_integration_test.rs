use std::io::Write;

use mud_driver::config::Config;

// =========================================================================
// Test 1: Load from YAML file
// =========================================================================

#[test]
fn load_full_config_from_yaml_file() {
    let yaml = r#"
server_name: TestMUD
ssh:
  host: 127.0.0.1
  port: 3333
  host_key: /etc/mud/host_key
http:
  host: 0.0.0.0
  port: 9090
  enabled: true
  session_secret: supersecret
world:
  path: /var/mud/world
  git_path: /var/mud/git
tick:
  interval: 0.5
database:
  host: db.example.com
  port: 5433
  admin_user: mud_root
  admin_password: secret123
  driver_db: mud_prod
  encryption_key: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
adapters:
  ruby:
    enabled: true
    command: /usr/local/bin/ruby
    adapter_path: /opt/mud/adapter/bin/mud-adapter
"#;

    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("config.yml");
    {
        let mut f = std::fs::File::create(&config_path).expect("create config file");
        f.write_all(yaml.as_bytes()).expect("write config file");
    }

    let cfg = Config::load(&config_path).expect("load config");

    assert_eq!(cfg.server_name, "TestMUD");

    // SSH
    assert_eq!(cfg.ssh.host, "127.0.0.1");
    assert_eq!(cfg.ssh.port, 3333);
    assert_eq!(cfg.ssh.host_key.as_deref(), Some("/etc/mud/host_key"));

    // HTTP
    assert_eq!(cfg.http.host, "0.0.0.0");
    assert_eq!(cfg.http.port, 9090);
    assert!(cfg.http.enabled);
    assert_eq!(cfg.http.session_secret.as_deref(), Some("supersecret"));

    // World
    assert_eq!(cfg.world.path, "/var/mud/world");
    assert_eq!(cfg.world.git_path, "/var/mud/git");

    // Tick
    assert!((cfg.tick.interval - 0.5).abs() < f64::EPSILON);

    // Database
    assert_eq!(cfg.database.host, "db.example.com");
    assert_eq!(cfg.database.port, 5433);
    assert_eq!(cfg.database.admin_user, "mud_root");
    assert_eq!(
        cfg.database.admin_password.as_deref(),
        Some("secret123")
    );
    assert_eq!(cfg.database.driver_db, "mud_prod");

    // Adapters
    let ruby = cfg.adapters.ruby.expect("ruby adapter config");
    assert!(ruby.enabled);
    assert_eq!(ruby.command, "/usr/local/bin/ruby");
    assert_eq!(ruby.adapter_path, "/opt/mud/adapter/bin/mud-adapter");
}

#[test]
fn load_partial_yaml_uses_defaults_for_missing_fields() {
    let yaml = "server_name: PartialMUD\n";

    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("partial.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let cfg = Config::load(&config_path).expect("load config");

    assert_eq!(cfg.server_name, "PartialMUD");
    // All other fields should have their defaults
    assert_eq!(cfg.ssh.host, "0.0.0.0");
    assert_eq!(cfg.ssh.port, 2222);
    assert!(cfg.ssh.host_key.is_none());
    assert_eq!(cfg.http.port, 8080);
    assert!(!cfg.http.enabled);
    assert_eq!(cfg.world.data_path, "data");
    assert_eq!(cfg.world.path, "world");
    assert_eq!(cfg.world.git_path, "git-server");
    assert!((cfg.tick.interval - 1.0).abs() < f64::EPSILON);
    assert_eq!(cfg.database.host, "localhost");
    assert_eq!(cfg.database.port, 5432);
    assert!(cfg.adapters.ruby.is_none());
}

// =========================================================================
// Test 2: Load with missing file returns defaults
// =========================================================================

#[test]
fn load_nonexistent_file_returns_defaults() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("does_not_exist.yml");

    let cfg = Config::load(&config_path).expect("load config with missing file");

    assert_eq!(cfg.server_name, "MUD Driver");
    assert_eq!(cfg.ssh.port, 2222);
    assert_eq!(cfg.http.port, 8080);
    assert_eq!(cfg.world.data_path, "data");
    assert_eq!(cfg.world.path, "world");
    assert!((cfg.tick.interval - 1.0).abs() < f64::EPSILON);
}

// =========================================================================
// Test 3: Validation failures
// =========================================================================

#[test]
fn validation_rejects_zero_ssh_port() {
    let yaml = "ssh:\n  port: 0\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("bad_ssh.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("ssh.port"),
        "error should mention ssh.port, got: {err}"
    );
}

#[test]
fn validation_rejects_zero_http_port() {
    let yaml = "http:\n  port: 0\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("bad_http.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("http.port"),
        "error should mention http.port, got: {err}"
    );
}

#[test]
fn validation_rejects_zero_tick_interval() {
    let yaml = "tick:\n  interval: 0\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("bad_tick.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("tick.interval"),
        "error should mention tick.interval, got: {err}"
    );
}

#[test]
fn validation_rejects_negative_tick_interval() {
    let yaml = "tick:\n  interval: -1.0\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("bad_tick_neg.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("tick.interval"),
        "error should mention tick.interval, got: {err}"
    );
}

#[test]
fn validation_rejects_empty_world_path() {
    let yaml = "world:\n  path: ''\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("bad_world.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("world.path"),
        "error should mention world.path, got: {err}"
    );
}

#[test]
fn validation_rejects_empty_git_path() {
    let yaml = "world:\n  git_path: ''\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("bad_git.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("world.git_path"),
        "error should mention world.git_path, got: {err}"
    );
}

#[test]
fn validation_rejects_zero_database_port() {
    let yaml = "database:\n  port: 0\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("bad_db.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("database.port"),
        "error should mention database.port, got: {err}"
    );
}

#[test]
fn malformed_yaml_returns_parse_error() {
    let yaml = "server_name: [invalid yaml\n  broken: {{{\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("malformed.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("YAML")
            || err.to_string().contains("parse")
            || err.to_string().contains("yaml"),
        "error should mention YAML parsing, got: {err}"
    );
}

#[test]
fn wrong_type_in_yaml_returns_error() {
    // ssh.port should be a number, not a string
    let yaml = "ssh:\n  port: not_a_number\n";
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("wrong_type.yml");
    std::fs::write(&config_path, yaml).expect("write config");

    let result = Config::load(&config_path);
    assert!(result.is_err(), "expected error for wrong type, got Ok");
}

#[test]
fn empty_yaml_file_uses_all_defaults() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let config_path = dir.path().join("empty.yml");
    std::fs::write(&config_path, "").expect("write empty file");

    let cfg = Config::load(&config_path).expect("load empty config");

    assert_eq!(cfg.server_name, "MUD Driver");
    assert_eq!(cfg.ssh.port, 2222);
    assert_eq!(cfg.http.port, 8080);
    assert!(!cfg.http.enabled);
    assert_eq!(cfg.world.data_path, "data");
    assert_eq!(cfg.world.path, "world");
    assert!((cfg.tick.interval - 1.0).abs() < f64::EPSILON);
}
