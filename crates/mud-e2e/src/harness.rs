use std::sync::LazyLock;
use std::time::{Duration, Instant};

use testcontainers::core::{IntoContainerPort, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::io::AsyncBufReadExt;

/// Name of the Docker image built from Dockerfile.e2e
const IMAGE_NAME: &str = "mud-driver-e2e";
const IMAGE_TAG: &str = "latest";

/// Print a timestamped log line showing elapsed seconds since `t0`.
fn log(t0: Instant, msg: &str) {
    let elapsed = t0.elapsed().as_secs_f64();
    eprintln!("[e2e +{elapsed:7.2}s] {msg}");
}

/// Build the musl binary and Docker image once per process. The binary is
/// statically linked (no glibc dependency) so it runs in any Linux container.
/// Subsequent calls from other test binaries hit Docker's layer cache.
static IMAGE_BUILT: LazyLock<()> = LazyLock::new(|| {
    let t0 = Instant::now();
    let project_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    // Clean up stale networks from previous test runs
    let _ = std::process::Command::new("docker")
        .args(["network", "prune", "-f", "--filter", "label!=keep"])
        .status();

    // Build static musl binaries (no-op if already up to date)
    log(
        t0,
        "Building mud-driver + mud-adapter-lpc (musl release)...",
    );
    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--release",
            "--bin",
            "mud-driver",
            "--bin",
            "mud-adapter-lpc",
            "--target",
            "x86_64-unknown-linux-musl",
        ])
        .current_dir(project_root)
        .status()
        .expect("failed to run cargo build");
    assert!(
        status.success(),
        "cargo build --release --target musl failed"
    );
    log(t0, "Musl binaries built");

    // Build JVM adapter: launcher JAR and publish MOP/stdlib to local Maven
    log(t0, "Building JVM adapter...");
    let jvm_dir = project_root.join("adapters/jvm");
    let gradlew = jvm_dir.join("gradlew");
    if gradlew.exists() {
        let status = std::process::Command::new(&gradlew)
            .args([
                ":mud-mop-jvm:publishToMavenLocal",
                ":stdlib:publishToMavenLocal",
                ":launcher:jar",
                "--no-daemon",
            ])
            .current_dir(&jvm_dir)
            .status()
            .expect("failed to run gradlew");
        assert!(status.success(), "JVM adapter build failed");

        // Copy local Maven artifacts into a directory Docker can COPY
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let m2_mud = std::path::PathBuf::from(&home).join(".m2/repository/mud");
        let local_m2 = project_root.join("adapters/jvm/local-m2/mud");
        if m2_mud.exists() {
            // Remove stale copy
            let _ = std::fs::remove_dir_all(&local_m2);
            copy_dir_recursive(&m2_mud, &local_m2);
        }
        log(t0, "JVM adapter built");
    } else {
        log(t0, "No JVM adapter gradlew found, skipping JVM build");
    }

    // Build Docker image (copies pre-built binary, adapters, and dependencies)
    log(t0, "Building Docker image...");
    let status = std::process::Command::new("docker")
        .args([
            "build",
            "-f",
            "Dockerfile.e2e",
            "-t",
            &format!("{IMAGE_NAME}:{IMAGE_TAG}"),
            ".",
        ])
        .current_dir(project_root)
        .status()
        .expect("failed to run docker build");
    assert!(status.success(), "docker build failed");
    log(t0, "Docker image built");
});

/// Recursively copy a directory tree (used to stage Maven artifacts for Docker COPY).
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let ty = entry.file_type().expect("file type");
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            std::fs::copy(entry.path(), &dst_path).expect("copy file");
        }
    }
}

/// A unique ID for naming Docker resources in this test instance.
fn unique_id() -> String {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    format!("e2e-{}-{}", std::process::id(), t.subsec_nanos())
}

pub struct TestServer {
    pub http_port: u16,
    pub client: reqwest::Client,
    pub network_name: String,
    // Drop order matters: containers first, then network, then config dir.
    _pg: ContainerAsync<Postgres>,
    _driver: ContainerAsync<GenericImage>,
    _config_dir: tempfile::TempDir,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with_adapters(true, true, false).await
    }

    /// Start a server with only the Ruby adapter enabled (no JVM adapter process).
    /// JVM templates should still be available via disk scanning.
    pub async fn start_ruby_only() -> Self {
        Self::start_with_adapters(true, false, false).await
    }

    /// Start a server with Ruby and LPC adapters enabled (no JVM).
    pub async fn start_with_lpc() -> Self {
        Self::start_with_adapters(true, true, true).await
    }

    pub async fn start_with_adapters(
        ruby_enabled: bool,
        jvm_enabled: bool,
        lpc_enabled: bool,
    ) -> Self {
        let t0 = Instant::now();

        // Ensure image is built
        LazyLock::force(&IMAGE_BUILT);
        log(t0, "Image ready");

        let id = unique_id();
        let network_name = format!("mud-{id}");
        let pg_name = format!("pg-{id}");

        // Create a Docker network
        log(t0, "Creating Docker network...");
        let status = std::process::Command::new("docker")
            .args(["network", "create", &network_name])
            .status()
            .expect("failed to create docker network");
        assert!(status.success(), "docker network create failed");

        // Start PostgreSQL with a known container name on the network
        log(t0, "Starting PostgreSQL...");
        let pg = Postgres::default()
            .with_network(&network_name)
            .with_container_name(&pg_name)
            .start()
            .await
            .expect("start PostgreSQL container");
        log(t0, "PostgreSQL started");

        // Generate config YAML — use pg container name as DB host
        let config_yaml = format!(
            r#"server_name: "E2E Test Server"
ssh:
  host: "0.0.0.0"
  port: 2222
http:
  host: "0.0.0.0"
  port: 8080
  enabled: true
database:
  host: "{pg_name}"
  port: 5432
  admin_user: "postgres"
  admin_password: "postgres"
  driver_db: "mud_driver"
  stdlib_db: "mud_stdlib"
  encryption_key: "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2"
adapters:
  ruby:
    enabled: {ruby_enabled}
    command: "ruby"
    adapter_path: "adapters/ruby/bin/mud-adapter"
  jvm:
    enabled: {jvm_enabled}
    command: "java"
    adapter_path: "adapters/jvm/launcher.jar"
  lpc:
    enabled: {lpc_enabled}
    command: "mud-adapter-lpc"
    adapter_path: "adapters/lpc"
"#
        );

        // Write config to a temp file for mounting
        let config_dir = tempfile::tempdir().expect("create temp dir for config");
        let config_path = config_dir.path().join("config.yml");
        std::fs::write(&config_path, &config_yaml).expect("write config");

        // Start mud-driver container WITHOUT wait_for — we stream logs manually
        // Note: GenericImage methods (with_exposed_port) must be called before
        // ImageExt methods (with_network, with_mount) because ImageExt methods
        // consume self and return ContainerRequest which doesn't have those methods.
        log(t0, "Starting mud-driver container...");
        let driver: ContainerAsync<GenericImage> = GenericImage::new(IMAGE_NAME, IMAGE_TAG)
            .with_exposed_port(8080.tcp())
            .with_exposed_port(2222.tcp())
            .with_network(&network_name)
            .with_mount(Mount::bind_mount(
                config_path.to_str().unwrap(),
                "/app/config.yml",
            ))
            .start()
            .await
            .expect("start mud-driver container");
        log(t0, "mud-driver container started");

        // Stream stdout and stderr to test output.
        // Watch stdout for "Server ready" and stderr for "Portal web server started".
        let stdout = driver.stdout(true);
        let (driver_ready_tx, driver_ready_rx) = tokio::sync::oneshot::channel::<()>();
        let stdout_t0 = t0;
        tokio::spawn(async move {
            let mut lines = stdout.lines();
            let mut tx = Some(driver_ready_tx);
            while let Ok(Some(line)) = lines.next_line().await {
                log(stdout_t0, &format!("[stdout] {line}"));
                if line.contains("Server ready") {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(());
                    }
                }
            }
        });

        let stderr = driver.stderr(true);
        let (portal_ready_tx, portal_ready_rx) = tokio::sync::oneshot::channel::<()>();
        let stderr_t0 = t0;
        tokio::spawn(async move {
            let mut lines = stderr.lines();
            let mut tx = Some(portal_ready_tx);
            while let Ok(Some(line)) = lines.next_line().await {
                log(stderr_t0, &format!("[stderr] {line}"));
                if line.contains("Portal web server started") {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(());
                    }
                }
            }
        });

        // Wait for both driver and portal to be ready
        log(t0, "Waiting for driver + portal ready...");
        let timeout = Duration::from_secs(120);
        if tokio::time::timeout(timeout, driver_ready_rx)
            .await
            .is_err()
        {
            panic!(
                "Timed out after {timeout:?} waiting for 'Server ready' in mud-driver stdout. \
                 Check log lines above for errors."
            );
        }
        log(t0, "Driver ready");
        if tokio::time::timeout(timeout, portal_ready_rx)
            .await
            .is_err()
        {
            panic!(
                "Timed out after {timeout:?} waiting for 'Portal web server started' in stderr. \
                 Check log lines above for errors."
            );
        }
        log(t0, "Portal ready");

        let http_port: u16 = driver
            .get_host_port_ipv4(8080.tcp())
            .await
            .expect("get HTTP port");
        log(t0, &format!("HTTP port mapped to {http_port}"));

        // Build HTTP client with cookie jar
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();

        // Poll until the HTTP server responds (portal ready)
        log(
            t0,
            &format!("Polling http://127.0.0.1:{http_port}/account/login ..."),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            if tokio::time::Instant::now() > deadline {
                panic!("Timed out waiting for HTTP server to respond on port {http_port}");
            }
            match client
                .get(format!("http://127.0.0.1:{http_port}/account/login"))
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status != 502 {
                        log(t0, &format!("HTTP responded {status}, ready!"));
                        break;
                    }
                }
                Err(e) => {
                    log(t0, &format!("HTTP error: {e:#}"));
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Self {
            http_port,
            client,
            _pg: pg,
            _driver: driver,
            network_name,
            _config_dir: config_dir,
        }
    }

    /// Build a full URL from a path (e.g. "/account/login")
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.http_port, path)
    }

    /// Create a fresh HTTP client (new cookie jar) for multi-user scenarios
    pub fn new_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    }

    /// Register a user (client will have session cookie set after this)
    pub async fn register_user(
        &self,
        client: &reqwest::Client,
        username: &str,
        password: &str,
        character: &str,
    ) {
        let resp = client
            .post(self.url("/account/register"))
            .form(&[
                ("username", username),
                ("password", password),
                ("character", character),
            ])
            .send()
            .await
            .expect("register request");
        assert_eq!(resp.status(), 302, "registration should redirect");
    }

    /// Login an existing user, returns the response status
    pub async fn login_user(
        &self,
        client: &reqwest::Client,
        username: &str,
        password: &str,
    ) -> reqwest::StatusCode {
        let resp = client
            .post(self.url("/account/login"))
            .form(&[("username", username), ("password", password)])
            .send()
            .await
            .expect("login request");
        resp.status()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Clean up the Docker network
        let _ = std::process::Command::new("docker")
            .args(["network", "rm", &self.network_name])
            .status();
    }
}

/// Poll a URL until the response body contains the expected string.
pub async fn poll_until_contains(
    client: &reqwest::Client,
    url: &str,
    expected: &str,
    timeout: Duration,
) -> String {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_status = None;
    let mut last_body = String::new();
    loop {
        if tokio::time::Instant::now() > deadline {
            let body_preview: String = last_body.chars().take(500).collect();
            panic!(
                "Timed out waiting for '{expected}' at {url}. \
                 Last status: {last_status:?}, body: '{body_preview}'"
            );
        }
        if let Ok(resp) = client.get(url).send().await {
            last_status = Some(resp.status());
            if resp.status() == 200 {
                let body = resp.text().await.unwrap_or_default();
                if body.contains(expected) {
                    return body;
                }
                last_body = body;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
