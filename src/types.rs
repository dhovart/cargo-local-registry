use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use reqwest::Client;

pub const DEFAULT_REFRESH_TTL_SECS: u64 = 15 * 60; // 15 minutes

#[derive(Clone)]
pub struct CachedIndex {
    pub content: bytes::Bytes,
    pub last_check: Instant,
}

#[derive(Clone)]
pub struct ExecutionControl {
    pub registry_path: PathBuf,
    pub server_url: String,
    pub reqwest_client: Client,
    pub enable_proxy: bool,
    pub clean: bool,
    pub index_cache: Arc<RwLock<HashMap<String, CachedIndex>>>,
    pub cache_ttl: Duration,
}
