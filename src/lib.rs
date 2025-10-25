use std::path::PathBuf;

use axum::{
    Json, Router, extract::Path as AxumPath, http::StatusCode, response::Response, routing::get,
};
use cargo::util::GlobalContext;
use cargo::util::errors::*;

pub async fn serve_registry(
    host: String,
    port: u16,
    path: String,
    _registry_url: Option<String>,
    _include_git: bool,
    _remove_previously_synced: bool,
    _config: &GlobalContext,
) -> CargoResult<()> {
    let registry_path = PathBuf::from(path);
    let server_url = format!("http://{}:{}", host, port);

    let app = Router::new()
        .route("/index/config.json", get(serve_config))
        .route("/index/{crate_name}", get(serve_index))
        .route("/{filename}", get(serve_crate_file))
        .fallback(serve_file)
        .with_state((registry_path, server_url));

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", host, port))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}:{}: {}", host, port, e))?;

    tracing::info!("Serving registry on http://{}:{}", host, port);

    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

    Ok(())
}

async fn serve_config(
    axum::extract::State((_, server_url)): axum::extract::State<(PathBuf, String)>,
) -> Json<serde_json::Value> {
    tracing::info!("Serving config.json");
    let config = serde_json::json!({
        "dl": format!("{}/{{crate}}-{{version}}.crate", server_url),
        "api": server_url
    });
    tracing::debug!(
        "Config response: {}",
        serde_json::to_string_pretty(&config).unwrap()
    );
    Json(config)
}

async fn serve_index(
    axum::extract::State((registry_path, _)): axum::extract::State<(PathBuf, String)>,
    AxumPath(crate_name): AxumPath<String>,
) -> Result<Response, StatusCode> {
    tracing::info!("Serving index for crate: {}", crate_name);
    let crate_name = crate_name.to_lowercase();
    let index_path = match crate_name.len() {
        1 => registry_path.join("index").join("1").join(&crate_name),
        2 => registry_path.join("index").join("2").join(&crate_name),
        3 => registry_path
            .join("index")
            .join("3")
            .join(&crate_name[..1])
            .join(&crate_name),
        _ => registry_path
            .join("index")
            .join(&crate_name[..2])
            .join(&crate_name[2..4])
            .join(&crate_name),
    };

    tracing::debug!("Looking for index file at: {}", index_path.display());

    match std::fs::read(&index_path) {
        Ok(content) => {
            tracing::info!(
                "Successfully served index for {}, {} bytes",
                crate_name,
                content.len()
            );
            let mut response = Response::new(axum::body::Body::from(content));
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                "text/plain".parse().unwrap(),
            );
            Ok(response)
        }
        Err(e) => {
            tracing::warn!("Failed to read index file for {}: {}", crate_name, e);
            Err(StatusCode::NOT_FOUND)
        }
    }
}

async fn serve_crate_file(
    axum::extract::State((registry_path, _)): axum::extract::State<(PathBuf, String)>,
    AxumPath(filename): AxumPath<String>,
) -> Result<Response, StatusCode> {
    if filename.ends_with(".crate") {
        tracing::info!("Serving crate file: {}", filename);
        let crate_path = registry_path.join(&filename);

        tracing::debug!("Looking for crate file at: {}", crate_path.display());

        match std::fs::read(&crate_path) {
            Ok(content) => {
                tracing::info!(
                    "Successfully served crate file {}, {} bytes",
                    filename,
                    content.len()
                );
                let mut response = Response::new(axum::body::Body::from(content));
                response.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    "application/octet-stream".parse().unwrap(),
                );
                Ok(response)
            }
            Err(e) => {
                tracing::warn!("Failed to read crate file {}: {}", filename, e);
                Err(StatusCode::NOT_FOUND)
            }
        }
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn serve_file(
    axum::extract::State((registry_path, _)): axum::extract::State<(PathBuf, String)>,
    uri: axum::http::Uri,
) -> Result<Response, StatusCode> {
    let file_path = uri.path().trim_start_matches('/');
    tracing::info!("Fallback file request for: {}", file_path);
    let full_path = registry_path.join(file_path);

    tracing::debug!("Looking for file at: {}", full_path.display());

    if !full_path.starts_with(&registry_path) {
        return Err(StatusCode::FORBIDDEN);
    }

    match std::fs::read(&full_path) {
        Ok(content) => {
            let content_len = content.len();
            let mut response = Response::new(axum::body::Body::from(content));

            if let Some(ext) = full_path.extension().and_then(|e| e.to_str()) {
                let content_type = match ext {
                    "json" => "application/json",
                    "tar" | "gz" => "application/gzip",
                    _ => "application/octet-stream",
                };

                response.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    content_type.parse().unwrap(),
                );
            }

            tracing::info!(
                "Successfully served file: {}, {} bytes",
                file_path,
                content_len
            );
            Ok(response)
        }
        Err(e) => {
            tracing::warn!("Failed to read file {}: {}", file_path, e);
            Err(StatusCode::NOT_FOUND)
        }
    }
}
