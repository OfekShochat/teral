use serde_derive::Deserialize;
use std::{fs::read, net::SocketAddr, sync::Arc};

use crate::storage::{RocksdbStorage, Storage};

#[derive(Deserialize)]
pub struct TeralConfig {
    pub storage: StorageConfig,
    pub identity: IdentityConfig,
    pub network: NetworkConfig,
    pub contracts_exec: ContractExecConfig,
}

impl TeralConfig {
    pub fn read(path: &str) -> Self {
        let bytes = read(path).expect("Could not read config file");
        toml::from_slice(&bytes).expect("Config error")
    }

    pub fn load_storage(&self) -> Option<Arc<dyn Storage>> {
        match self.storage.backend {
            #[cfg(feature = "rocksdb-backend")]
            DbBackend::Rocksdb => Some(RocksdbStorage::load(&self.storage)),
            // _ => None,
        }
    }

    // pub fn get_scheduler(&self) -> Option<Arc<dyn LeaderSchedule>> {
    //     match self.network.leader_schedule {
    //         LeaderScheduleBackend::StdRng => Some(StdRngSchedule::new()),
    //     }
    // }
}

#[derive(Deserialize)]
pub struct NetworkConfig {
    pub addr: String,
    pub known_nodes: Vec<SocketAddr>,
    // pub leader_schedule: LeaderScheduleBackend,
}

#[derive(Deserialize)]
pub enum LeaderScheduleBackend {
    #[serde(rename = "stdrng")]
    StdRng,
}

#[derive(Deserialize)]
pub struct StorageConfig {
    pub backend: DbBackend,
    pub path: String,
    pub log_history: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: DbBackend::Rocksdb,
            path: String::from("db/"),
            log_history: 1,
        }
    }
}

#[derive(Deserialize)]
pub struct IdentityConfig {
    pub path: String,
}

#[derive(Deserialize)]
pub struct ContractExecConfig {
    pub threads: usize,
}

#[derive(Deserialize)]
pub enum DbBackend {
    #[serde(rename = "rocksdb")]
    Rocksdb,
}
