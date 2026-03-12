//! Integration tests for the smart HTTP git protocol endpoints.
//!
//! These tests verify the handlers defined in `web::git_http`, covering:
//! - `GET /:ns/:name.git/info/refs?service=git-upload-pack` (ref advertisement)
//! - `POST /:ns/:name.git/git-upload-pack` (fetch)
//! - `POST /:ns/:name.git/git-receive-pack` (push)
//!
//! All tests require Docker (PostgreSQL via testcontainers).
//! Run with: `cargo test -p mud-driver --test http_git_test`

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

use mud_driver::git::repo_manager::{AccessLevel, RepoManager};
use mud_driver::git::workspace::Workspace;
use mud_driver::persistence::player_store::PlayerStore;
use mud_driver::web::build_log::BuildLog;
use mud_driver::web::git_http::git_http_routes;
use mud_driver::web::server::AppState;

// ---------------------------------------------------------------------------
// Helper: encode HTTP Basic credentials
// ---------------------------------------------------------------------------

fn basic_auth(username: &str, password: &str) -> String {
    use base64::Engine;
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    format!("Basic {}", encoded)
}

// ---------------------------------------------------------------------------
// Test harness with PostgreSQL-backed PlayerStore (testcontainers)
// ---------------------------------------------------------------------------

struct TestHarness {
    _temp_dir: tempfile::TempDir,
    app: Router,
    repo_manager: Arc<RepoManager>,
    #[allow(dead_code)]
    workspace: Arc<Workspace>,
    player_store: Arc<PlayerStore>,
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn build_harness() -> TestHarness {
    let dir = tempfile::tempdir().expect("create temp dir");
    let repos_path = dir.path().join("repos");
    let world_path = dir.path().join("world");
    std::fs::create_dir_all(&repos_path).unwrap();
    std::fs::create_dir_all(&world_path).unwrap();

    let repo_manager = Arc::new(RepoManager::new(repos_path));
    let workspace = Arc::new(Workspace::new(world_path, Arc::clone(&repo_manager)));

    // Start PostgreSQL container.
    let container = Postgres::default()
        .start()
        .await
        .expect("start PostgreSQL container");

    let host_port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get PostgreSQL port");

    let db_url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        host_port
    );

    // Connect and create stdlib tables.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connect to test PostgreSQL");

    for sql in &[
        "CREATE TABLE IF NOT EXISTS players (
            id VARCHAR PRIMARY KEY,
            password_hash VARCHAR NOT NULL,
            role VARCHAR NOT NULL DEFAULT 'builder',
            active_character VARCHAR,
            builder_character_id INTEGER,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        )",
        "CREATE TABLE IF NOT EXISTS access_tokens (
            id SERIAL PRIMARY KEY,
            player_id VARCHAR NOT NULL REFERENCES players(id),
            name VARCHAR NOT NULL,
            token_prefix VARCHAR NOT NULL,
            token_hash VARCHAR NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
            last_used_at TIMESTAMPTZ
        )",
        "CREATE TABLE IF NOT EXISTS sessions (
            id SERIAL PRIMARY KEY,
            player_id VARCHAR NOT NULL REFERENCES players(id),
            token VARCHAR NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        )",
        "CREATE TABLE IF NOT EXISTS characters (
            id SERIAL PRIMARY KEY,
            player_id VARCHAR NOT NULL REFERENCES players(id),
            name VARCHAR NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        )",
    ] {
        sqlx::query(sql).execute(&pool).await.expect("create table");
    }

    let player_store = Arc::new(PlayerStore::new(pool));
    let templates = Arc::new(tera::Tera::default());

    let state = AppState {
        player_store: Arc::clone(&player_store),
        repo_manager: Arc::clone(&repo_manager),
        workspace: Arc::clone(&workspace),
        templates,
        ai_key_store: None,
        skills_service: None,
        http_client: reqwest::Client::new(),
        portal_socket: "/tmp/mud-portal.sock".into(),
        anthropic_base_url: None,
        build_manager: None,
        build_log: Arc::new(BuildLog::new(200)),
        mop_rpc: None,
        area_web_sockets: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        area_templates: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        loaded_areas: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
    };

    let app = Router::new()
        .nest("/git", git_http_routes())
        .with_state(state);

    TestHarness {
        _temp_dir: dir,
        app,
        repo_manager,
        workspace,
        player_store,
        _container: container,
    }
}

// ===========================================================================
// info/refs tests
// ===========================================================================

#[tokio::test]

async fn info_refs_no_auth_returns_401() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let www_auth = resp.headers().get("www-authenticate");
    assert!(www_auth.is_some(), "must include WWW-Authenticate header");
    assert!(
        www_auth.unwrap().to_str().unwrap().contains("Basic"),
        "must specify Basic scheme"
    );
}

#[tokio::test]

async fn info_refs_wrong_password_returns_401() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("correctpass").unwrap();
    h.player_store.create("alice", &hash).await.unwrap();

    h.repo_manager
        .grant_access("testns", "village", "alice", AccessLevel::ReadWrite)
        .unwrap();

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("alice", "wrongpassword"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]

async fn info_refs_nonexistent_user_returns_401() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("nobody", "nopass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]

async fn info_refs_valid_owner_returns_200_with_pktline() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("secret123").unwrap();
    h.player_store.create("testns", &hash).await.unwrap();

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("testns", "secret123"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Content-Type
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/x-git-upload-pack-advertisement");

    // Cache-Control
    let cc = resp
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(cc, "no-cache");

    // Body pkt-line format
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body);

    assert!(
        body_str.contains("# service=git-upload-pack"),
        "body must contain service header"
    );
    assert!(body_str.contains("0000"), "body must contain flush packet");
    assert!(
        body_str.contains("HEAD"),
        "body must advertise HEAD reference"
    );
    assert!(
        body_str.contains("refs/heads/main"),
        "body must advertise refs/heads/main"
    );
    assert!(
        body_str.contains("refs/heads/develop"),
        "body must advertise refs/heads/develop"
    );
    assert!(
        body_str.contains("report-status"),
        "body must include capabilities"
    );
}

#[tokio::test]

async fn info_refs_collaborator_read_access_returns_200() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("readerpass").unwrap();
    h.player_store.create("reader", &hash).await.unwrap();

    h.repo_manager
        .grant_access("testns", "village", "reader", AccessLevel::ReadOnly)
        .unwrap();

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("reader", "readerpass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]

async fn info_refs_unauthorized_user_returns_403() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("strangerpass").unwrap();
    h.player_store.create("stranger", &hash).await.unwrap();

    // stranger has no ACL entry for this repo.
    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("stranger", "strangerpass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]

async fn info_refs_receive_pack_readonly_user_returns_403() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("readerpass").unwrap();
    h.player_store.create("reader", &hash).await.unwrap();

    h.repo_manager
        .grant_access("testns", "village", "reader", AccessLevel::ReadOnly)
        .unwrap();

    // git-receive-pack requires ReadWrite.
    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-receive-pack")
        .header("Authorization", basic_auth("reader", "readerpass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]

async fn info_refs_receive_pack_owner_returns_200() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("ownerpass").unwrap();
    h.player_store.create("testns", &hash).await.unwrap();

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-receive-pack")
        .header("Authorization", basic_auth("testns", "ownerpass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/x-git-receive-pack-advertisement");
}

#[tokio::test]

async fn info_refs_invalid_service_returns_400() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("testns", &hash).await.unwrap();

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-bogus")
        .header("Authorization", basic_auth("testns", "pass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]

async fn info_refs_nonexistent_repo_returns_404() {
    let h = build_harness().await;

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("alice", &hash).await.unwrap();

    let req = Request::builder()
        .uri("/git/testns/nonexistent.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("alice", "pass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// git-upload-pack (fetch) tests
// ===========================================================================

#[tokio::test]

async fn upload_pack_no_auth_returns_401() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/git/testns/village.git/git-upload-pack")
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]

async fn upload_pack_valid_auth_returns_200() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("testns", &hash).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/git/testns/village.git/git-upload-pack")
        .header("Authorization", basic_auth("testns", "pass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/x-git-upload-pack-result");
}

#[tokio::test]

async fn upload_pack_unauthorized_user_returns_403() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("stranger", &hash).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/git/testns/village.git/git-upload-pack")
        .header("Authorization", basic_auth("stranger", "pass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ===========================================================================
// git-receive-pack (push) tests
// ===========================================================================

#[tokio::test]

async fn receive_pack_no_auth_returns_401() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/git/testns/village.git/git-receive-pack")
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]

async fn receive_pack_readonly_returns_403() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("reader", &hash).await.unwrap();

    h.repo_manager
        .grant_access("testns", "village", "reader", AccessLevel::ReadOnly)
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/git/testns/village.git/git-receive-pack")
        .header("Authorization", basic_auth("reader", "pass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]

async fn receive_pack_owner_returns_200() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("testns", &hash).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/git/testns/village.git/git-receive-pack")
        .header("Authorization", basic_auth("testns", "pass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/x-git-receive-pack-result");
}

#[tokio::test]

async fn receive_pack_readwrite_collaborator_returns_200() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("writer", &hash).await.unwrap();

    h.repo_manager
        .grant_access("testns", "village", "writer", AccessLevel::ReadWrite)
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/git/testns/village.git/git-receive-pack")
        .header("Authorization", basic_auth("writer", "pass"))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ===========================================================================
// PAT (personal access token) authentication
// ===========================================================================

#[tokio::test]

async fn pat_auth_works_for_info_refs() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("testns", &hash).await.unwrap();

    let (token, _prefix) = h
        .player_store
        .create_access_token("testns", "git-token")
        .await
        .unwrap();

    assert!(token.starts_with("mud_"), "PAT must start with mud_");

    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header("Authorization", basic_auth("testns", &token))
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]

async fn pat_auth_wrong_token_returns_401() {
    let h = build_harness().await;

    h.repo_manager
        .create_repo("testns", "village", true, None)
        .unwrap();

    let hash = PlayerStore::hash_password("pass").unwrap();
    h.player_store.create("testns", &hash).await.unwrap();

    // Use a fake PAT that starts with "mud_" but is not valid.
    let req = Request::builder()
        .uri("/git/testns/village.git/info/refs?service=git-upload-pack")
        .header(
            "Authorization",
            basic_auth("testns", "mud_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        )
        .body(Body::empty())
        .unwrap();

    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
