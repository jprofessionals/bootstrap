use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use mud_driver::web::build_log::BuildLog;
use mud_driver::web::project::project_routes;

/// Create a temp directory with the following structure:
///
/// ```text
/// world/ns/myarea/          (empty area workspace)
/// build-cache/ns-myarea/dist/
///   index.html              "<html><body>SPA</body></html>"
///   assets/main.js          "console.log('hello')"
/// ```
fn setup_dirs(tmp: &std::path::Path) -> (PathBuf, PathBuf) {
    let world = tmp.join("world");
    let cache = tmp.join("build-cache");

    // Area workspace (empty, just needs to exist)
    std::fs::create_dir_all(world.join("ns/myarea")).unwrap();

    // SPA dist
    let dist = cache.join("ns-myarea/dist");
    std::fs::create_dir_all(dist.join("assets")).unwrap();
    std::fs::write(dist.join("index.html"), "<html><body>SPA</body></html>").unwrap();
    std::fs::write(dist.join("assets/main.js"), "console.log('hello')").unwrap();

    (world, cache)
}

fn build_app(world: PathBuf, cache: PathBuf) -> axum::Router {
    let build_log = Arc::new(BuildLog::new(100));
    project_routes(world, cache, build_log, None, "/nonexistent.sock".into())
}

#[tokio::test]
async fn test_serve_spa_index() {
    let tmp = tempfile::tempdir().unwrap();
    let (world, cache) = setup_dirs(tmp.path());
    let app = build_app(world, cache);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ns/myarea/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("SPA"));
    assert!(text.contains("<html>"));
}

#[tokio::test]
async fn test_serve_spa_asset() {
    let tmp = tempfile::tempdir().unwrap();
    let (world, cache) = setup_dirs(tmp.path());
    let app = build_app(world, cache);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ns/myarea/assets/main.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let content_type = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.contains("javascript"),
        "expected javascript content-type, got: {}",
        content_type
    );

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(text, "console.log('hello')");
}

#[tokio::test]
async fn test_spa_fallback_to_index() {
    let tmp = tempfile::tempdir().unwrap();
    let (world, cache) = setup_dirs(tmp.path());
    let app = build_app(world, cache);

    // Request a path that doesn't exist as a file — should serve index.html
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ns/myarea/settings/profile")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("SPA"), "SPA fallback should serve index.html");
}

#[tokio::test]
async fn test_404_for_unknown_area() {
    let tmp = tempfile::tempdir().unwrap();
    let (world, cache) = setup_dirs(tmp.path());
    let app = build_app(world, cache);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ns/nonexistent/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_serve_tera_template() {
    let tmp = tempfile::tempdir().unwrap();
    let world = tmp.path().join("world");
    let templates = world.join("ns/templated/web/templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(
        templates.join("index.html"),
        "<h1>Hello {{ area_name }}</h1>",
    )
    .unwrap();

    let build_log = Arc::new(BuildLog::new(100));
    let router = project_routes(world, tmp.path().join("build-cache"), build_log, None, "/nonexistent.sock".into());

    let resp = router
        .clone()
        .oneshot(
            Request::get("/ns/templated/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("Hello templated"),
        "expected rendered template with area_name, got: {}",
        text
    );
}

#[tokio::test]
async fn test_path_traversal_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let (world, cache) = setup_dirs(tmp.path());
    let app = build_app(world, cache);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ns/myarea/../../etc/passwd")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
