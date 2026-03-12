use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::process::Command;
use tracing::{error, info};

use super::build_log::{BuildLog, LogLevel};

pub struct BuildManager {
    build_log: Arc<BuildLog>,
    cache_path: PathBuf,
}

impl BuildManager {
    pub fn new(build_log: Arc<BuildLog>, cache_path: PathBuf) -> Self {
        // Clear stale builds on startup
        if cache_path.exists() {
            if let Err(e) = std::fs::remove_dir_all(&cache_path) {
                error!(path = %cache_path.display(), error = %e, "failed to clear build cache");
            } else {
                info!(path = %cache_path.display(), "cleared build cache on startup");
            }
        }
        let _ = std::fs::create_dir_all(&cache_path);
        Self {
            build_log,
            cache_path,
        }
    }

    /// Returns true if the area has a SPA setup (web/src/package.json exists).
    pub fn is_spa(area_path: &Path) -> bool {
        area_path.join("web/src/package.json").is_file()
    }

    /// Returns true if the area has template files (web/templates/ contains at least one file).
    pub fn is_template(area_path: &Path) -> bool {
        let tpl_dir = area_path.join("web/templates");
        tpl_dir.is_dir()
            && std::fs::read_dir(&tpl_dir)
                .map(|mut entries| entries.any(|e| e.map(|e| e.path().is_file()).unwrap_or(false)))
                .unwrap_or(false)
    }

    /// Returns the build output directory for a given area key.
    pub fn build_dir(&self, area_key: &str) -> PathBuf {
        let sanitised = area_key.replace('/', "-");
        self.cache_path.join(sanitised)
    }

    /// Spawns a background tokio task to build the SPA for the given area.
    pub fn trigger_build(&self, area_key: String, area_path: PathBuf, base_url: String) {
        let build_dir = self.build_dir(&area_key);
        let build_log = Arc::clone(&self.build_log);
        tokio::spawn(async move {
            if let Err(e) =
                run_spa_build(&area_key, &area_path, &build_dir, &base_url, &build_log).await
            {
                error!(area_key = %area_key, error = %e, "SPA build failed");
                build_log.append(
                    &area_key,
                    LogLevel::Error,
                    "build",
                    &format!("Build failed: {e}"),
                );
            }
        });
    }
}

async fn run_spa_build(
    area_key: &str,
    area_path: &Path,
    build_dir: &Path,
    base_url: &str,
    build_log: &BuildLog,
) -> anyhow::Result<()> {
    let src_dir = area_path.join("web/src");
    if !src_dir.is_dir() {
        anyhow::bail!("web/src directory not found in area {area_key}");
    }

    build_log.append(area_key, LogLevel::Info, "build", "Starting SPA build");

    // Clean and copy source to build dir
    if build_dir.exists() {
        std::fs::remove_dir_all(build_dir)?;
    }
    std::fs::create_dir_all(build_dir)?;
    copy_dir_recursive(&src_dir, build_dir)?;

    info!(area_key = %area_key, "Copied web/src to build dir");
    build_log.append(
        area_key,
        LogLevel::Info,
        "build",
        "Copied web/src to build directory",
    );

    // npm install
    let output = Command::new("npm")
        .arg("install")
        .current_dir(build_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        build_log.append(
            area_key,
            LogLevel::Error,
            "npm-install",
            &format!("npm install failed: {stderr}"),
        );
        anyhow::bail!("npm install failed: {stderr}");
    }
    build_log.append(
        area_key,
        LogLevel::Info,
        "npm-install",
        "npm install succeeded",
    );

    // npx vite build
    let output = Command::new("npx")
        .args(["vite", "build", "--base", base_url])
        .env("MUD_BASE_URL", base_url)
        .current_dir(build_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        build_log.append(
            area_key,
            LogLevel::Error,
            "vite-build",
            &format!("vite build failed: {stderr}"),
        );
        anyhow::bail!("vite build failed: {stderr}");
    }
    build_log.append(
        area_key,
        LogLevel::Info,
        "vite-build",
        "vite build succeeded",
    );

    // Inject MUD global into index.html
    inject_mud_global(build_dir, base_url)?;
    build_log.append(
        area_key,
        LogLevel::Info,
        "build",
        "Build completed successfully",
    );

    info!(area_key = %area_key, "SPA build completed");
    Ok(())
}

fn inject_mud_global(build_dir: &Path, base_url: &str) -> anyhow::Result<()> {
    let index_path = build_dir.join("dist/index.html");
    if !index_path.is_file() {
        anyhow::bail!("dist/index.html not found after vite build");
    }

    let html = std::fs::read_to_string(&index_path)?;
    let script = format!("<script>window.__MUD__={{baseUrl:\"{base_url}\"}}</script>");
    let patched = html.replacen("<head>", &format!("<head>{script}"), 1);
    std::fs::write(&index_path, patched)?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}
