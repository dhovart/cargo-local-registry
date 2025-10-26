mod crates;
mod index;
mod parsing;
mod types;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::{
    Json, Router, extract::Path as AxumPath, http::StatusCode, response::Response, routing::get,
};
use cargo::util::errors::*;
use reqwest::Client;

use parsing::parse_crate_filename;
pub use types::{CachedIndex, DEFAULT_REFRESH_TTL_SECS, ExecutionControl};

pub async fn serve_registry(
    host: String,
    port: u16,
    path: String,
    enable_proxy: bool,
    clean: bool,
) -> CargoResult<()> {
    let registry_path = PathBuf::from(path);
    let server_url = format!("http://{}:{}", host, port);
    let client = Client::new();

    let state = ExecutionControl {
        registry_path: registry_path.clone(),
        server_url: server_url.clone(),
        reqwest_client: client.clone(),
        enable_proxy,
        clean,
        index_cache: Arc::new(RwLock::new(HashMap::new())),
        cache_ttl: Duration::from_secs(DEFAULT_REFRESH_TTL_SECS),
    };

    let app = Router::new()
        .route("/index/config.json", get(serve_config))
        .route("/index/{*path}", get(serve_index_generic))
        .route("/{filename}", get(serve_crate_file))
        .fallback(serve_file)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("{}:{}", host, port))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}:{}: {}", host, port, e))?;

    tracing::info!("Serving registry on http://{}:{}", host, port);

    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

    Ok(())
}

pub async fn serve_config(
    axum::extract::State(ExecutionControl { server_url, .. }): axum::extract::State<
        ExecutionControl,
    >,
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

pub async fn serve_index_generic(
    axum::extract::State(ExecutionControl {
        registry_path,
        reqwest_client,
        enable_proxy,
        index_cache,
        cache_ttl,
        ..
    }): axum::extract::State<ExecutionControl>,
    AxumPath(path): AxumPath<String>,
) -> Result<Response, StatusCode> {
    let crate_name = path.split('/').next_back().unwrap_or(&path).to_string();
    tracing::info!(
        "Serving index for crate: {} (from path: {})",
        crate_name,
        path
    );
    let crate_name = crate_name.to_lowercase();
    let index_path = index::get_index_path(&registry_path, &crate_name);

    tracing::debug!("Looking for index file at: {}", index_path.display());

    if enable_proxy {
        let should_try_refresh = if let Ok(cache) = index_cache.read() {
            if let Some(cached) = cache.get(&crate_name) {
                let since_last_check = cached.last_check.elapsed();
                if since_last_check < cache_ttl {
                    tracing::info!(
                        "Serving {} from cache (last checked {:?} ago)",
                        crate_name,
                        since_last_check
                    );
                    let mut response =
                        Response::new(axum::body::Body::from(cached.content.clone()));
                    response.headers_mut().insert(
                        axum::http::header::CONTENT_TYPE,
                        "text/plain".parse().unwrap(),
                    );
                    return Ok(response);
                } else {
                    tracing::info!(
                        "Checking if crates.io is responsive for {} (last checked {:?} ago)",
                        crate_name,
                        since_last_check
                    );
                    true
                }
            } else {
                true
            }
        } else {
            true
        };

        if should_try_refresh {
            tracing::info!("Trying quick fetch from crates.io for {}", crate_name);

            let crates_io_url = index::get_crates_io_index_url(&crate_name);

            let fast_fail_duration = Duration::from_millis(500);

            let request = reqwest_client
                .get(&crates_io_url)
                .timeout(fast_fail_duration);

            match request.send().await {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(content) => {
                        tracing::info!(
                            "Successfully fetched fresh index for {} from crates.io in <500ms, {} bytes - caching",
                            crate_name,
                            content.len()
                        );

                        if let Ok(mut cache) = index_cache.write() {
                            cache.insert(
                                crate_name.clone(),
                                CachedIndex {
                                    content: content.clone(),
                                    last_check: Instant::now(),
                                },
                            );
                            tracing::debug!("Cached fresh index for {}", crate_name);
                        }

                        let mut response = Response::new(axum::body::Body::from(content));
                        response.headers_mut().insert(
                            axum::http::header::CONTENT_TYPE,
                            "text/plain".parse().unwrap(),
                        );
                        return Ok(response);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read response from crates.io: {}", e);
                    }
                },
                Ok(response) => {
                    tracing::warn!(
                        "crates.io returned status {}: {}",
                        response.status(),
                        crate_name
                    );
                }
                Err(e) => {
                    tracing::info!(
                        "crates.io timeout or error for {} ({}), using cache",
                        crate_name,
                        e
                    );
                }
            }

            if let Ok(mut cache) = index_cache.write()
                && let Some(cached) = cache.get_mut(&crate_name) {
                    cached.last_check = Instant::now();
                    tracing::debug!("Updated last_check for {}", crate_name);
                }
        }
    }

    match std::fs::read(&index_path) {
        Ok(content) => {
            tracing::info!(
                "Serving cached index for {}, {} bytes",
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
            tracing::warn!(
                "No local index file for {} and proxy failed: {}",
                crate_name,
                e
            );

            if enable_proxy {
                tracing::info!(
                    "Attempting to proxy index for {} from crates.io",
                    crate_name
                );

                let crates_io_url = index::get_crates_io_index_url(&crate_name);

                match reqwest_client.get(&crates_io_url).send().await {
                    Ok(response) if response.status().is_success() => {
                        match response.bytes().await {
                            Ok(content) => {
                                tracing::info!(
                                    "Successfully proxied index for {} from crates.io, {} bytes",
                                    crate_name,
                                    content.len()
                                );

                                tracing::info!("Caching full index for {} locally", crate_name);

                                if let Some(parent) = index_path.parent()
                                    && let Err(e) = std::fs::create_dir_all(parent) {
                                        tracing::warn!("Failed to create index directory: {}", e);
                                    }

                                if let Err(e) = std::fs::write(&index_path, &content) {
                                    tracing::warn!("Failed to cache index file locally: {}", e);
                                } else {
                                    tracing::info!("Successfully cached index for {}", crate_name);
                                }

                                let mut response = Response::new(axum::body::Body::from(content));
                                response.headers_mut().insert(
                                    axum::http::header::CONTENT_TYPE,
                                    "text/plain".parse().unwrap(),
                                );
                                Ok(response)
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to read response body from crates.io: {}",
                                    e
                                );
                                Err(StatusCode::INTERNAL_SERVER_ERROR)
                            }
                        }
                    }
                    Ok(response) => {
                        tracing::warn!(
                            "crates.io returned status {}: {}",
                            response.status(),
                            crate_name
                        );
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

pub async fn serve_crate_file(
    axum::extract::State(state): axum::extract::State<ExecutionControl>,
    AxumPath(filename): AxumPath<String>,
) -> Result<Response, StatusCode> {
    if filename.ends_with(".crate") {
        tracing::info!("Serving crate file: {}", filename);
        let crate_path = state.registry_path.join(&filename);

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

                if state.enable_proxy {
                    tracing::info!("Attempting to proxy crate file {} from crates.io", filename);

                    let crate_info = parse_crate_filename(&filename);

                    let crates_io_url = if let Some((crate_name, version)) = crate_info {
                        format!(
                            "https://crates.io/api/v1/crates/{}/{}/download",
                            crate_name, version
                        )
                    } else {
                        tracing::error!("Invalid crate filename format: {}", filename);
                        return Err(StatusCode::BAD_REQUEST);
                    };

                    match state.reqwest_client.get(&crates_io_url).send().await {
                        Ok(response) if response.status().is_success() => {
                            match response.bytes().await {
                                Ok(content) => {
                                    tracing::info!(
                                        "Successfully proxied crate file {} from crates.io, {} bytes",
                                        filename,
                                        content.len()
                                    );

                                    if let Some((crate_name, version)) = crate_info {
                                        if state.clean {
                                            crates::remove_prior_versions(
                                                &state.registry_path,
                                                crate_name,
                                                version,
                                            );
                                        }

                                        if let Err(e) = std::fs::write(&crate_path, &content) {
                                            tracing::warn!(
                                                "Failed to cache crate file locally: {}",
                                                e
                                            );
                                        }

                                        cache_specific_index_version(
                                            &state.reqwest_client,
                                            &state.registry_path,
                                            crate_name,
                                            version,
                                            state.clean,
                                        )
                                        .await;
                                    } else if let Err(e) = std::fs::write(&crate_path, &content) {
                                        tracing::warn!(
                                            "Failed to cache crate file locally: {}",
                                            e
                                        );
                                    }

                                    let mut response =
                                        Response::new(axum::body::Body::from(content));
                                    response.headers_mut().insert(
                                        axum::http::header::CONTENT_TYPE,
                                        "application/octet-stream".parse().unwrap(),
                                    );
                                    Ok(response)
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to read crate response body from crates.io: {}",
                                        e
                                    );
                                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                                }
                            }
                        }
                        Ok(response) => {
                            tracing::warn!(
                                "crates.io returned status {} for crate {}",
                                response.status(),
                                filename
                            );
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

pub async fn serve_file(
    axum::extract::State(ExecutionControl { registry_path, .. }): axum::extract::State<
        ExecutionControl,
    >,
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

async fn cache_specific_index_version(
    client: &Client,
    registry_path: &Path,
    crate_name: &str,
    version: &str,
    clean: bool,
) {
    tracing::info!("Caching index entry for {}:{}", crate_name, version);

    let index_path = index::get_index_path(registry_path, crate_name);
    let crates_io_url = index::get_crates_io_index_url(crate_name);

    match client.get(&crates_io_url).send().await {
        Ok(response) if response.status().is_success() => {
            if let Ok(content) = response.bytes().await {
                let content_str = String::from_utf8_lossy(&content);

                for line in content_str.lines() {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line)
                        && let Some(version_str) = parsed.get("vers").and_then(|v| v.as_str())
                        && version_str == version
                    {
                        let mut cached_content = String::new();

                        if clean {
                            cached_content.push_str(line);
                            cached_content.push('\n');
                        } else {
                            if let Ok(existing) = std::fs::read_to_string(&index_path) {
                                cached_content = existing;
                            }

                            if !cached_content.contains(&format!("\"vers\":\"{}\"", version)) {
                                cached_content.push_str(line);
                                cached_content.push('\n');
                            } else {
                                return;
                            }
                        }

                        if let Some(parent) = index_path.parent()
                            && let Err(e) = std::fs::create_dir_all(parent)
                        {
                            tracing::warn!("Failed to create index directory: {}", e);
                            return;
                        }

                        if let Err(e) = std::fs::write(&index_path, cached_content.as_bytes()) {
                            tracing::warn!("Failed to cache index entry: {}", e);
                        } else {
                            tracing::info!(
                                "Successfully cached index entry for {}:{}",
                                crate_name,
                                version
                            );
                        }
                        return;
                    }
                }
                tracing::warn!("Version {} not found in index for {}", version, crate_name);
            }
        }
        Ok(response) => {
            tracing::warn!(
                "Failed to fetch index for caching: status {}",
                response.status()
            );
        }
        Err(e) => {
            tracing::error!("Failed to fetch index for caching: {}", e);
        }
    }
}
