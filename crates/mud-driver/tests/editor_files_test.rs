use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use mud_driver::web::editor_files::editor_file_routes;

/// Helper: create a temp world dir with a repo at `ns/name@dev/` containing some files.
fn setup_world() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let world_path = dir.path().to_path_buf();

    // Create ns/testarea@dev with some files
    let area_dev = world_path.join("ns").join("testarea@dev");
    std::fs::create_dir_all(area_dev.join("rooms")).unwrap();
    std::fs::write(area_dev.join("main.lua"), "-- main file\n").unwrap();
    std::fs::write(area_dev.join("rooms/hall.lua"), "-- hall\n").unwrap();

    // Create a .git dir to test exclusion
    let git_dir = area_dev.join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

    (dir, world_path)
}

fn app(world_path: PathBuf) -> axum::Router {
    editor_file_routes(world_path, None, String::new())
}

fn json_body(content: &str) -> Body {
    Body::from(serde_json::json!({ "content": content }).to_string())
}

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[tokio::test]
async fn test_list_files() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path);

    let resp = app
        .oneshot(
            Request::get("/files?repo=ns/testarea")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_json(resp).await;
    let files = body["files"].as_array().unwrap();

    // Should contain main.lua and rooms/hall.lua
    let paths: Vec<&str> = files.iter().map(|f| f.as_str().unwrap()).collect();
    assert!(
        paths.contains(&"main.lua"),
        "expected main.lua in {:?}",
        paths
    );
    assert!(
        paths.contains(&"rooms/hall.lua"),
        "expected rooms/hall.lua in {:?}",
        paths
    );

    // Should NOT contain .git files
    for p in &paths {
        assert!(!p.starts_with(".git"), ".git path leaked: {}", p);
    }
}

#[tokio::test]
async fn test_read_file() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path);

    let resp = app
        .oneshot(
            Request::get("/files/main.lua?repo=ns/testarea")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_json(resp).await;
    assert_eq!(body["content"].as_str().unwrap(), "-- main file\n");
    assert_eq!(body["path"].as_str().unwrap(), "main.lua");
}

#[tokio::test]
async fn test_write_file() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path.clone());

    let resp = app
        .oneshot(
            Request::put("/files/main.lua?repo=ns/testarea")
                .header("content-type", "application/json")
                .body(json_body("-- updated\n"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Verify the file was actually written
    let content = std::fs::read_to_string(world_path.join("ns/testarea@dev/main.lua")).unwrap();
    assert_eq!(content, "-- updated\n");
}

#[tokio::test]
async fn test_create_file() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path.clone());

    let resp = app
        .oneshot(
            Request::post("/files/newfile.lua?repo=ns/testarea")
                .header("content-type", "application/json")
                .body(json_body("-- brand new\n"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);

    let content = std::fs::read_to_string(world_path.join("ns/testarea@dev/newfile.lua")).unwrap();
    assert_eq!(content, "-- brand new\n");
}

#[tokio::test]
async fn test_create_file_conflict() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path);

    let resp = app
        .oneshot(
            Request::post("/files/main.lua?repo=ns/testarea")
                .header("content-type", "application/json")
                .body(json_body("-- conflict\n"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_delete_file() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path.clone());

    let resp = app
        .oneshot(
            Request::delete("/files/main.lua?repo=ns/testarea")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!world_path.join("ns/testarea@dev/main.lua").exists());
}

#[tokio::test]
async fn test_path_traversal_blocked() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path);

    let resp = app
        .oneshot(
            Request::get("/files/../../etc/passwd?repo=ns/testarea")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should be 403 or 400 — anything that blocks the traversal
    assert!(
        resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::BAD_REQUEST,
        "expected 403 or 400, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_git_dir_blocked() {
    let (_dir, world_path) = setup_world();
    let app = app(world_path);

    let resp = app
        .oneshot(
            Request::get("/files/.git/HEAD?repo=ns/testarea")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
