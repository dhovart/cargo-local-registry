use std::fs::{self, File};
use std::io::Write;

use axum_test::TestServer;
use cargo_local_registry::{ExecutionControl, serve_registry};
use reqwest::Client;
use tempfile::TempDir;

async fn create_test_registry() -> TempDir {
    let registry = TempDir::new().unwrap();

    fs::create_dir_all(registry.path().join("index/1")).unwrap();
    fs::create_dir_all(registry.path().join("index/2")).unwrap();
    fs::create_dir_all(registry.path().join("index/3/s")).unwrap();
    fs::create_dir_all(registry.path().join("index/se/rd")).unwrap();

    let serde_index_path = registry.path().join("index/se/rd/serde");
    let mut serde_index = File::create(&serde_index_path).unwrap();
    writeln!(
        serde_index,
        r#"{{"name":"serde","vers":"1.0.130","deps":[],"cksum":"f12d906a1a742b6bd55d37d7b5685e0b46f3b8190d4190dbf3944a0bcc8bb25f","features":{{"derive":["serde_derive"],"std":[],"unstable":[],"alloc":[],"rc":[]}},"yanked":false,"links":null}}"#
    ).unwrap();

    let a_index_path = registry.path().join("index/1/a");
    let mut a_index = File::create(&a_index_path).unwrap();
    writeln!(
        a_index,
        r#"{{"name":"a","vers":"0.1.0","deps":[],"cksum":"abcd1234","features":{{}},"yanked":false,"links":null}}"#
    ).unwrap();

    let serde_crate = registry.path().join("serde-1.0.130.crate");
    File::create(&serde_crate)
        .unwrap()
        .write_all(b"fake crate content for serde")
        .unwrap();

    let a_crate = registry.path().join("a-0.1.0.crate");
    File::create(&a_crate)
        .unwrap()
        .write_all(b"fake crate content for a")
        .unwrap();

    registry
}

fn create_test_app(
    registry_path: std::path::PathBuf,
    enable_proxy: bool,
    clean: bool,
) -> axum::Router {
    let client = Client::new();
    let state = ExecutionControl {
        registry_path,
        server_url: "http://127.0.0.1:8080".to_string(),
        reqwest_client: client,
        enable_proxy,
        clean,
    };

    axum::Router::new()
        .route(
            "/index/config.json",
            axum::routing::get(cargo_local_registry::serve_config),
        )
        .route(
            "/index/{crate_name}",
            axum::routing::get(cargo_local_registry::serve_index),
        )
        .route(
            "/{filename}",
            axum::routing::get(cargo_local_registry::serve_crate_file),
        )
        .fallback(cargo_local_registry::serve_file)
        .with_state(state)
}

#[tokio::test]
async fn test_config_json_endpoint() {
    let registry = create_test_registry().await;
    let app = create_test_app(registry.path().to_path_buf(), false, false);
    let server = TestServer::new(app).unwrap();

    let response = server.get("/index/config.json").await;
    response.assert_status_ok();

    let json: serde_json::Value = response.json();
    assert_eq!(json["dl"], "http://127.0.0.1:8080/{crate}-{version}.crate");
    assert_eq!(json["api"], "http://127.0.0.1:8080");
}

#[tokio::test]
async fn test_serve_index_1_char() {
    let registry = create_test_registry().await;
    let app = create_test_app(registry.path().to_path_buf(), false, false);
    let server = TestServer::new(app).unwrap();

    let response = server.get("/index/a").await;
    response.assert_status_ok();
    response.assert_header("content-type", "text/plain");

    let content = response.text();
    assert!(content.contains(r#""name":"a""#));
    assert!(content.contains(r#""vers":"0.1.0""#));
}

#[tokio::test]
async fn test_serve_index_4_plus_chars() {
    let registry = create_test_registry().await;
    let app = create_test_app(registry.path().to_path_buf(), false, false);
    let server = TestServer::new(app).unwrap();

    let response = server.get("/index/serde").await;
    response.assert_status_ok();
    response.assert_header("content-type", "text/plain");

    let content = response.text();
    assert!(content.contains(r#""name":"serde""#));
    assert!(content.contains(r#""vers":"1.0.130""#));
}

#[tokio::test]
async fn test_serve_crate_file() {
    let registry = create_test_registry().await;
    let app = create_test_app(registry.path().to_path_buf(), false, false);
    let server = TestServer::new(app).unwrap();

    let response = server.get("/serde-1.0.130.crate").await;
    response.assert_status_ok();
    response.assert_header("content-type", "application/octet-stream");

    let content = response.as_bytes();
    assert_eq!(content.as_ref(), b"fake crate content for serde");
}

#[tokio::test]
async fn test_index_not_found_no_proxy() {
    let registry = create_test_registry().await;
    let app = create_test_app(registry.path().to_path_buf(), false, false);
    let server = TestServer::new(app).unwrap();

    let response = server.get("/index/nonexistent").await;
    response.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_crate_file_not_found_no_proxy() {
    let registry = create_test_registry().await;
    let app = create_test_app(registry.path().to_path_buf(), false, false);
    let server = TestServer::new(app).unwrap();

    let response = server.get("/nonexistent-1.0.0.crate").await;
    response.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_invalid_crate_filename() {
    let registry = create_test_registry().await;
    let app = create_test_app(registry.path().to_path_buf(), true, false);
    let server = TestServer::new(app).unwrap();

    let response = server.get("/invalid-file").await;
    response.assert_status(axum::http::StatusCode::NOT_FOUND);

    let response = server.get("/invalid.crate").await;
    response.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_serve_registry_function_signature() {
    let _registry = create_test_registry().await;
    let registry_path = _registry.path().to_string_lossy().to_string();

    let _future = serve_registry("127.0.0.1".to_string(), 8080, registry_path, false, false);
}
