use serde_derive::Deserialize;
use std::{fs::read, net::SocketAddr};

pub fn config_from_file(path: &str) -> TeralConfig {
    let bytes = read(path).expect("Could not read config file");
    toml::from_slice(&bytes).expect("Config error")
}

#[derive(Deserialize)]
pub struct TeralConfig {
    pub storage: StorageConfig,
    pub identity: IdentityConfig,
    pub network: NetworkConfig,
}

#[derive(Deserialize)]
pub struct NetworkConfig {
    pub addr: String,
    pub known_nodes: Vec<SocketAddr>,
}

#[derive(Deserialize)]
pub struct StorageConfig {
    pub backend: DbBackend,
    pub path: String,
}

#[derive(Deserialize)]
pub struct IdentityConfig {
    pub path: String,
}

#[derive(Deserialize)]
pub enum DbBackend {
    #[serde(rename = "rocksdb")]
    Rocksdb,
}
