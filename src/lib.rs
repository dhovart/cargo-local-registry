use std::path::PathBuf;

use axum::{
    Json, Router, extract::Path as AxumPath, http::StatusCode, response::Response, routing::get,
};
use cargo::util::GlobalContext;
use cargo::util::errors::*;
use reqwest::Client;

pub async fn serve_registry(
    host: String,
    port: u16,
    path: String,
    _registry_url: Option<String>,
    _include_git: bool,
    _remove_previously_synced: bool,
    enable_proxy: bool,
    _config: &GlobalContext,
) -> CargoResult<()> {
    let registry_path = PathBuf::from(path);
    let server_url = format!("http://{}:{}", host, port);
    let client = Client::new();

    let app = Router::new()
        .route("/index/config.json", get(serve_config))
        .route("/index/{crate_name}", get(serve_index))
        .route("/{filename}", get(serve_crate_file))
        .fallback(serve_file)
        .with_state((registry_path, server_url, client, enable_proxy));

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
    axum::extract::State((_, server_url, _, _)): axum::extract::State<(PathBuf, String, Client, bool)>,
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
    axum::extract::State((registry_path, _, client, enable_proxy)): axum::extract::State<(PathBuf, String, Client, bool)>,
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
            tracing::warn!("Failed to read local index file for {}: {}", crate_name, e);

            // If proxy is enabled, try to fetch from crates.io
            if enable_proxy {
                tracing::info!("Attempting to proxy index for {} from crates.io", crate_name);

                let crates_io_url = match crate_name.len() {
                    1 => format!("https://index.crates.io/1/{}", crate_name),
                    2 => format!("https://index.crates.io/2/{}", crate_name),
                    3 => format!("https://index.crates.io/3/{}/{}", &crate_name[..1], crate_name),
                    _ => format!("https://index.crates.io/{}/{}/{}", &crate_name[..2], &crate_name[2..4], crate_name),
                };

                match client.get(&crates_io_url).send().await {
                    Ok(response) if response.status().is_success() => {
                        match response.bytes().await {
                            Ok(content) => {
                                tracing::info!(
                                    "Successfully proxied index for {} from crates.io, {} bytes",
                                    crate_name,
                                    content.len()
                                );

                                // Don't cache the index yet - we'll cache specific versions when .crate files are downloaded
                                tracing::debug!("Returning full index for dependency resolution, not caching yet");

                                // Return the full content to the client (for dependency resolution)
                                let mut response = Response::new(axum::body::Body::from(content));
                                response.headers_mut().insert(
                                    axum::http::header::CONTENT_TYPE,
                                    "text/plain".parse().unwrap(),
                                );
                                Ok(response)
                            }
                            Err(e) => {
                                tracing::error!("Failed to read response body from crates.io: {}", e);
                                Err(StatusCode::INTERNAL_SERVER_ERROR)
                            }
                        }
                    }
                    Ok(response) => {
                        tracing::warn!("crates.io returned status {}: {}", response.status(), crate_name);
                        Err(StatusCode::NOT_FOUND)
                    }
                    Err(e) => {
                        tracing::error!("Failed to proxy request to crates.io: {}", e);
                        Err(StatusCode::INTERNAL_SERVER_ERROR)
                    }
                }
            } else {
                Err(StatusCode::NOT_FOUND)
            }
        }
    }
}

async fn serve_crate_file(
    axum::extract::State((registry_path, _, client, enable_proxy)): axum::extract::State<(PathBuf, String, Client, bool)>,
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
                tracing::warn!("Failed to read local crate file {}: {}", filename, e);

                // If proxy is enabled, try to fetch from crates.io
                if enable_proxy {
                    tracing::info!("Attempting to proxy crate file {} from crates.io", filename);

                    // Parse the filename to extract crate name and version
                    // Format is expected to be {crate}-{version}.crate
                    let crate_info = if let Some(stripped) = filename.strip_suffix(".crate") {
                        // Find the last dash to separate name and version
                        if let Some(dash_pos) = stripped.rfind('-') {
                            let crate_name = &stripped[..dash_pos];
                            let version = &stripped[dash_pos + 1..];
                            Some((crate_name, version))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let crates_io_url = if let Some((crate_name, version)) = crate_info {
                        format!("https://crates.io/api/v1/crates/{}/{}/download", crate_name, version)
                    } else {
                        tracing::error!("Invalid crate filename format: {}", filename);
                        return Err(StatusCode::BAD_REQUEST);
                    };

                    match client.get(&crates_io_url).send().await {
                        Ok(response) if response.status().is_success() => {
                            match response.bytes().await {
                                Ok(content) => {
                                    tracing::info!(
                                        "Successfully proxied crate file {} from crates.io, {} bytes",
                                        filename,
                                        content.len()
                                    );

                                    // Save crate file to local registry for future use
                                    if let Err(e) = std::fs::write(&crate_path, &content) {
                                        tracing::warn!("Failed to cache crate file locally: {}", e);
                                    }

                                    // Now that we've downloaded this specific version, cache its index entry
                                    if let Some((crate_name, version)) = crate_info {
                                        cache_specific_index_version(&client, &registry_path, crate_name, version).await;
                                    }

                                    let mut response = Response::new(axum::body::Body::from(content));
                                    response.headers_mut().insert(
                                        axum::http::header::CONTENT_TYPE,
                                        "application/octet-stream".parse().unwrap(),
                                    );
                                    Ok(response)
                                }
                                Err(e) => {
                                    tracing::error!("Failed to read crate response body from crates.io: {}", e);
                                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                                }
                            }
                        }
                        Ok(response) => {
                            tracing::warn!("crates.io returned status {} for crate {}", response.status(), filename);
                            Err(StatusCode::NOT_FOUND)
                        }
                        Err(e) => {
                            tracing::error!("Failed to proxy crate request to crates.io: {}", e);
                            Err(StatusCode::INTERNAL_SERVER_ERROR)
                        }
                    }
                } else {
                    Err(StatusCode::NOT_FOUND)
                }
            }
        }
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn serve_file(
    axum::extract::State((registry_path, _, _, _)): axum::extract::State<(PathBuf, String, Client, bool)>,
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

async fn cache_specific_index_version(client: &Client, registry_path: &PathBuf, crate_name: &str, version: &str) {
    tracing::info!("Caching index entry for {}:{}", crate_name, version);

    // Construct the index file path
    let index_path = match crate_name.len() {
        1 => registry_path.join("index").join("1").join(crate_name),
        2 => registry_path.join("index").join("2").join(crate_name),
        3 => registry_path
            .join("index")
            .join("3")
            .join(&crate_name[..1])
            .join(crate_name),
        _ => registry_path
            .join("index")
            .join(&crate_name[..2])
            .join(&crate_name[2..4])
            .join(crate_name),
    };

    // Fetch the full index from crates.io to find this specific version
    let crates_io_url = match crate_name.len() {
        1 => format!("https://index.crates.io/1/{}", crate_name),
        2 => format!("https://index.crates.io/2/{}", crate_name),
        3 => format!("https://index.crates.io/3/{}/{}", &crate_name[..1], crate_name),
        _ => format!("https://index.crates.io/{}/{}/{}", &crate_name[..2], &crate_name[2..4], crate_name),
    };

    match client.get(&crates_io_url).send().await {
        Ok(response) if response.status().is_success() => {
            if let Ok(content) = response.bytes().await {
                let content_str = String::from_utf8_lossy(&content);

                // Find the specific version line
                for line in content_str.lines() {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) {
                        if let Some(version_str) = parsed.get("vers").and_then(|v| v.as_str()) {
                            if version_str == version {
                                // Found the version we want to cache
                                let mut cached_content = String::new();

                                // Read existing cached content if it exists
                                if let Ok(existing) = std::fs::read_to_string(&index_path) {
                                    cached_content = existing;
                                }

                                // Check if this version is already cached
                                if !cached_content.contains(&format!("\"vers\":\"{}\"", version)) {
                                    cached_content.push_str(line);
                                    cached_content.push('\n');

                                    // Create directory if needed
                                    if let Some(parent) = index_path.parent() {
                                        if let Err(e) = std::fs::create_dir_all(parent) {
                                            tracing::warn!("Failed to create index directory: {}", e);
                                            return;
                                        }
                                    }

                                    // Save the cached content
                                    if let Err(e) = std::fs::write(&index_path, cached_content.as_bytes()) {
                                        tracing::warn!("Failed to cache index entry: {}", e);
                                    } else {
                                        tracing::info!("Successfully cached index entry for {}:{}", crate_name, version);
                                    }
                                }
                                return;
                            }
                        }
                    }
                }
                tracing::warn!("Version {} not found in index for {}", version, crate_name);
            }
        }
        Ok(response) => {
            tracing::warn!("Failed to fetch index for caching: status {}", response.status());
        }
        Err(e) => {
            tracing::error!("Failed to fetch index for caching: {}", e);
        }
    }
}
