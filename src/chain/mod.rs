use std::sync::Arc;

use serde_json::Value;
use serde_derive::{Serialize, Deserialize};

use crate::storage::Storage;

#[derive(Serialize, Deserialize)]
struct ContractRecipt {
    contract_name: String, // NOTE: this will work when the contract is updated because the chain is evaluated from the start.
    contract_method: String,
    req: Value,
}

#[derive(Serialize, Deserialize)]
pub struct Block {
    digest: [u8; 32],
    previous_digest: [u8; 32],
    recipts: Vec<ContractRecipt>,
    time: i64,
}

struct BlockStorage {
    storage: Arc<dyn Storage>,
}

impl BlockStorage {
    fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    fn insert_block(&self, block: Block) {
        let serialized = serde_json::to_string(&block).unwrap();
        self.storage.set(&block.digest, serialized.as_bytes());
    }

    fn latest_block(&self) -> Option<Block> {
        let latest_hash = self.storage.get(b"latest_block")?;
        self.block_by_hash(&latest_hash)
    }

    fn block_by_hash(&self, hash: &[u8]) -> Option<Block> {
        let bytes = self.storage.get(&[b"block", hash].concat())?;
        serde_json::from_slice(&bytes).unwrap_or(None)
    }
}

pub struct Chain {
    storage: BlockStorage,

}
