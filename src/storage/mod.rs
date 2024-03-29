use std::sync::Arc;

pub trait Storage {
    fn load(config: &StorageConfig) -> Arc<Self>
    where
        Self: Sized;

    fn get(&self, key: &[u8]) -> Option<Vec<u8>>;

    fn delete(&self, key: &[u8]);

    fn delete_prefix(&self, prefix: &[u8]);

    fn set(&self, key: &[u8], value: &[u8]);

    fn get_or_set(&self, key: &[u8], alternative_value: &[u8]) -> Vec<u8>;
}

#[cfg(feature = "rocksdb-backend")]
use rocksdb::{Options, DB};

use crate::config::StorageConfig;

#[cfg(feature = "rocksdb-backend")]
pub struct RocksdbStorage {
    db: DB,
}

#[cfg(feature = "rocksdb-backend")]
impl Storage for RocksdbStorage {
    fn load(config: &StorageConfig) -> Arc<Self>
    where
        Self: Sized,
    {
        let mut options = Options::default();
        options.create_if_missing(true);
        options.set_keep_log_file_num(config.log_history);

        Arc::new(Self {
            db: DB::open(&options, &config.path).unwrap(),
        })
    }

    fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.db.get(key).unwrap()
    }

    fn delete(&self, key: &[u8]) {
        self.db.delete(key).unwrap();
    }

    fn delete_prefix(&self, prefix: &[u8]) {
        for key in self.db.prefix_iterator(prefix) {
            self.delete(&key.0);
        }
    }

    fn set(&self, key: &[u8], value: &[u8]) {
        self.db.put(key, value).unwrap();
    }

    fn get_or_set(&self, key: &[u8], alternative_value: &[u8]) -> Vec<u8> {
        if let Some(value) = self.get(key) {
            value
        } else {
            self.set(key, alternative_value);
            alternative_value.to_vec()
        }
    }
}
