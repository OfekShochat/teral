use std::sync::Arc;

use serde_json::Value;
use serde_derive::{Serialize, Deserialize};
use sha3::{Sha3_256, Digest};

use crate::storage::Storage;

fn hash_requests(recipts: &[ContractRecipt], time: i64, output: &mut [u8]) {
    let mut hasher = Sha3_256::new();
    recipts.into_iter().for_each(|req| {
        let mut s = String::with_capacity(50);
        s.push_str(&req.contract_name);
        s.push_str(&req.contract_method);
        s.push_str(&serde_json::to_string(&req.req).unwrap());
        // TODO: somehow make this with AsRef<[u8]>. Currently doing this does not work because
        // of ownership.

        hasher.update(s);
    });
    hasher.update(time.to_be_bytes());

    output.copy_from_slice(&hasher.finalize());
}

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