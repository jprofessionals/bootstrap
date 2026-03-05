use std::path::PathBuf;
use std::sync::Arc;

use mud_driver::web::build_log::BuildLog;
use mud_driver::web::build_manager::BuildManager;

#[test]
fn test_detect_spa_mode() {
    let dir = tempfile::tempdir().unwrap();

    // No web/src/package.json → not SPA
    assert!(!BuildManager::is_spa(dir.path()));

    // Create web/src/package.json → SPA
    let pkg_dir = dir.path().join("web/src");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("package.json"), "{}").unwrap();
    assert!(BuildManager::is_spa(dir.path()));
}

#[test]
fn test_detect_template_mode() {
    let dir = tempfile::tempdir().unwrap();

    // No web/templates/ → not template
    assert!(!BuildManager::is_template(dir.path()));

    // Empty web/templates/ → not template
    let tpl_dir = dir.path().join("web/templates");
    std::fs::create_dir_all(&tpl_dir).unwrap();
    assert!(!BuildManager::is_template(dir.path()));

    // web/templates/ with a file → template
    std::fs::write(tpl_dir.join("index.html"), "<html></html>").unwrap();
    assert!(BuildManager::is_template(dir.path()));
}

#[test]
fn test_build_dir_path() {
    let build_log = Arc::new(BuildLog::new(100));
    let cache_dir = tempfile::tempdir().unwrap();
    let cache_path = cache_dir.path().to_path_buf();
    let manager = BuildManager::new(build_log, cache_path.clone());

    assert_eq!(
        manager.build_dir("world/town"),
        cache_path.join("world-town")
    );
    assert_eq!(
        manager.build_dir("myarea"),
        cache_path.join("myarea")
    );
}
