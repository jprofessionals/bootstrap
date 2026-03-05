use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Top-level Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_name: String,
    pub ssh: SshConfig,
    pub http: HttpConfig,
    pub world: WorldConfig,
    pub tick: TickConfig,
    pub database: DatabaseConfig,
    pub adapters: AdaptersConfig,
    pub ai: AiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_name: "MUD Driver".into(),
            ssh: SshConfig::default(),
            http: HttpConfig::default(),
            world: WorldConfig::default(),
            tick: TickConfig::default(),
            database: DatabaseConfig::default(),
            adapters: AdaptersConfig::default(),
            ai: AiConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from a YAML file on disk.
    ///
    /// If the file does not exist, the defaults are used.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            tracing::warn!(?path, "Config file not found — using defaults");
            let cfg = Self::default();
            cfg.validate()?;
            return Ok(cfg);
        }

        let contents =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_yaml(&contents)
    }

    /// Parse a YAML string into a `Config` (useful for testing).
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let cfg: Self =
            serde_yaml::from_str(yaml).context("failed to parse YAML configuration")?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate semantic constraints that serde cannot express.
    pub fn validate(&self) -> Result<()> {
        if self.ssh.port == 0 {
            bail!("ssh.port must be > 0");
        }
        if self.http.port == 0 {
            bail!("http.port must be > 0");
        }
        if self.tick.interval <= 0.0 {
            bail!("tick.interval must be > 0");
        }
        if self.world.data_path.is_empty() {
            bail!("world.data_path must not be empty");
        }
        if self.world.path.is_empty() {
            bail!("world.path must not be empty");
        }
        if self.world.git_path.is_empty() {
            bail!("world.git_path must not be empty");
        }
        if self.database.port == 0 {
            bail!("database.port must be > 0");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SSH
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub host_key: Option<String>,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 2222,
            host_key: None,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub enabled: bool,
    pub session_secret: Option<String>,
    pub portal_socket: String,
    pub build_cache_path: String,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8080,
            enabled: false,
            session_secret: None,
            portal_socket: "/tmp/mud-portal.sock".into(),
            build_cache_path: "/tmp/mud-builder-cache".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// World
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WorldConfig {
    /// Root directory for all runtime data.
    pub data_path: String,
    /// Working directory checkouts (relative to `data_path` unless absolute).
    pub path: String,
    /// Bare git repositories (relative to `data_path` unless absolute).
    pub git_path: String,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            data_path: "data".into(),
            path: "world".into(),
            git_path: "git-server".into(),
        }
    }
}

impl WorldConfig {
    /// Resolve `path` relative to `data_path` (unless it is absolute).
    pub fn resolved_path(&self) -> std::path::PathBuf {
        let p = std::path::Path::new(&self.path);
        if p.is_absolute() { p.to_path_buf() } else { std::path::Path::new(&self.data_path).join(p) }
    }

    /// Resolve `git_path` relative to `data_path` (unless it is absolute).
    pub fn resolved_git_path(&self) -> std::path::PathBuf {
        let p = std::path::Path::new(&self.git_path);
        if p.is_absolute() { p.to_path_buf() } else { std::path::Path::new(&self.data_path).join(p) }
    }
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TickConfig {
    pub interval: f64,
}

impl Default for TickConfig {
    fn default() -> Self {
        Self { interval: 1.0 }
    }
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub admin_user: String,
    pub admin_password: Option<String>,
    pub driver_db: String,
    pub stdlib_db: String,
    pub encryption_key: Option<String>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 5432,
            admin_user: "mud_admin".into(),
            admin_password: None,
            driver_db: "mud_driver".into(),
            stdlib_db: "mud_stdlib".into(),
            encryption_key: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Adapters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AdaptersConfig {
    pub ruby: Option<RubyAdapterConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RubyAdapterConfig {
    pub enabled: bool,
    pub command: String,
    pub adapter_path: String,
}

impl Default for RubyAdapterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: "ruby".into(),
            adapter_path: "adapters/ruby/bin/mud-adapter".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// AI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub skill_repos: Vec<String>,
    pub skills_cache_dir: String,
    /// Override the base URL for the Anthropic API (e.g. a corporate proxy).
    pub anthropic_base_url: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            skill_repos: Vec::new(),
            skills_cache_dir: ".cache/ai-skills".into(),
            anthropic_base_url: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn parses_minimal_yaml() {
        let cfg = Config::from_yaml("server_name: TestMUD\n").unwrap();
        assert_eq!(cfg.server_name, "TestMUD");
        assert_eq!(cfg.ssh.port, 2222);
    }

    #[test]
    fn parses_empty_yaml() {
        let cfg = Config::from_yaml("").unwrap();
        assert_eq!(cfg.server_name, "MUD Driver");
    }

    #[test]
    fn parses_nested_override() {
        let yaml = r#"
ssh:
  port: 3333
tick:
  interval: 0.5
"#;
        let cfg = Config::from_yaml(yaml).unwrap();
        assert_eq!(cfg.ssh.port, 3333);
        assert!((cfg.tick.interval - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn rejects_zero_ssh_port() {
        let yaml = "ssh:\n  port: 0\n";
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("ssh.port"));
    }

    #[test]
    fn rejects_zero_tick_interval() {
        let yaml = "tick:\n  interval: 0\n";
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("tick.interval"));
    }

    #[test]
    fn rejects_empty_world_path() {
        let yaml = "world:\n  path: ''\n";
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("world.path"));
    }

    #[test]
    fn parses_ruby_adapter() {
        let yaml = r#"
adapters:
  ruby:
    enabled: false
    command: /usr/bin/ruby
    adapter_path: /opt/mud/adapter
"#;
        let cfg = Config::from_yaml(yaml).unwrap();
        let ruby = cfg.adapters.ruby.unwrap();
        assert!(!ruby.enabled);
        assert_eq!(ruby.command, "/usr/bin/ruby");
        assert_eq!(ruby.adapter_path, "/opt/mud/adapter");
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let cfg = Config::load(Path::new("/tmp/nonexistent_mud_config_12345.yml")).unwrap();
        assert_eq!(cfg.server_name, "MUD Driver");
    }

    #[test]
    fn rejects_zero_http_port() {
        let yaml = "http:\n  port: 0\n";
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("http.port"));
    }

    #[test]
    fn rejects_zero_database_port() {
        let yaml = "database:\n  port: 0\n";
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("database.port"));
    }

    #[test]
    fn rejects_empty_git_path() {
        let yaml = "world:\n  git_path: ''\n";
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("world.git_path"));
    }

    #[test]
    fn rejects_negative_tick_interval() {
        let yaml = "tick:\n  interval: -1.0\n";
        let err = Config::from_yaml(yaml).unwrap_err();
        assert!(err.to_string().contains("tick.interval"));
    }

    #[test]
    fn default_ssh_config() {
        let cfg = SshConfig::default();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 2222);
        assert!(cfg.host_key.is_none());
    }

    #[test]
    fn default_http_config() {
        let cfg = HttpConfig::default();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 8080);
        assert!(!cfg.enabled);
        assert!(cfg.session_secret.is_none());
    }

    #[test]
    fn default_world_config() {
        let cfg = WorldConfig::default();
        assert_eq!(cfg.data_path, "data");
        assert_eq!(cfg.path, "world");
        assert_eq!(cfg.git_path, "git-server");
        assert_eq!(cfg.resolved_path(), std::path::PathBuf::from("data/world"));
        assert_eq!(cfg.resolved_git_path(), std::path::PathBuf::from("data/git-server"));
    }

    #[test]
    fn default_tick_config() {
        let cfg = TickConfig::default();
        assert!((cfg.interval - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn default_database_config() {
        let cfg = DatabaseConfig::default();
        assert_eq!(cfg.host, "localhost");
        assert_eq!(cfg.port, 5432);
        assert_eq!(cfg.admin_user, "mud_admin");
        assert!(cfg.admin_password.is_none());
        assert_eq!(cfg.driver_db, "mud_driver");
        assert_eq!(cfg.stdlib_db, "mud_stdlib");
        assert!(cfg.encryption_key.is_none());
    }

    #[test]
    fn default_ruby_adapter_config() {
        let cfg = RubyAdapterConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.command, "ruby");
    }

    #[test]
    fn parses_full_config() {
        let yaml = r#"
server_name: VikingMUD
ssh:
  host: 127.0.0.1
  port: 4222
http:
  host: 127.0.0.1
  port: 9090
  enabled: true
  session_secret: my_secret
world:
  path: /data/world
  git_path: /data/git
tick:
  interval: 2.5
database:
  host: db.local
  port: 5433
  admin_user: admin
  admin_password: pass123
  driver_db: my_driver
  stdlib_db: my_stdlib
  encryption_key: aaaabbbbccccddddaaaabbbbccccdddd
"#;
        let cfg = Config::from_yaml(yaml).unwrap();
        assert_eq!(cfg.server_name, "VikingMUD");
        assert_eq!(cfg.ssh.host, "127.0.0.1");
        assert_eq!(cfg.ssh.port, 4222);
        assert_eq!(cfg.http.port, 9090);
        assert!(cfg.http.enabled);
        assert_eq!(cfg.http.session_secret.as_deref(), Some("my_secret"));
        assert_eq!(cfg.world.path, "/data/world");
        assert_eq!(cfg.world.git_path, "/data/git");
        assert!((cfg.tick.interval - 2.5).abs() < f64::EPSILON);
        assert_eq!(cfg.database.host, "db.local");
        assert_eq!(cfg.database.port, 5433);
        assert_eq!(cfg.database.admin_password.as_deref(), Some("pass123"));
    }

    #[test]
    fn adapters_config_default_has_no_ruby() {
        let cfg = AdaptersConfig::default();
        assert!(cfg.ruby.is_none());
    }

    #[test]
    fn parses_disabled_ruby_adapter() {
        let yaml = r#"
adapters:
  ruby:
    enabled: false
"#;
        let cfg = Config::from_yaml(yaml).unwrap();
        let ruby = cfg.adapters.ruby.unwrap();
        assert!(!ruby.enabled);
    }

    #[test]
    fn rejects_invalid_yaml() {
        let result = Config::from_yaml("{ invalid: yaml: here }}}");
        assert!(result.is_err());
    }
}
