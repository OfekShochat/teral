use std::{
    fmt::{self, Debug},
    sync::Arc,
};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use sha3::{Digest, Sha3_256};

use crate::{contracts::ContractRequest, storage::Storage};

fn hash_recipts(recipts: &[ContractRecipt], time: i64, output: &mut [u8]) {
    let mut hasher = Sha3_256::new();
    recipts.iter().for_each(|req| {
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

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractRecipt {
    contract_name: String, // NOTE: this will work when the contract is updated because the chain is evaluated from the start.
    contract_method: String,
    req: Value,
}

impl From<ContractRequest> for ContractRecipt {
    fn from(req: ContractRequest) -> Self {
        Self {
            contract_name: req.name,
            contract_method: req.method_name,
            req: req.req,
        }
    }
}

pub fn requests_to_recipts(req: Vec<ContractRequest>) -> Vec<ContractRecipt> {
    req.into_iter().map(|req| req.into()).collect()
}

#[derive(Serialize, Deserialize)]
pub struct Block {
    digest: [u8; 32],
    previous_digest: [u8; 32],
    recipts: Vec<ContractRecipt>,
    time: i64,
}

impl Block {
    pub fn with_transactions(transactions: Vec<ContractRecipt>) -> Self {
        Self {
            digest: [0; 32],
            previous_digest: [0; 32],
            recipts: transactions,
            time: Utc::now().timestamp_millis(),
        }
    }
}

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let time =
            DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(self.time / 1000, 0), Utc);
        f.debug_struct("Block")
            .field("digest", &base64::encode(self.digest))
            .field("previous_digest", &base64::encode(self.previous_digest))
            .field("time", &time)
            // .field("recipts", ) // TODO: somehow show something like [item1, ...] len: x
            .finish()
    }
}

struct BlockStorage {
    storage: Arc<dyn Storage>,
}

impl BlockStorage {
    fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }

    fn insert_block(&self, block: Block, set_latest: bool) {
        if set_latest {
            self.storage.set(b"latest_block", &block.digest);
        }
        let serialized = serde_json::to_string(&block).unwrap();
        self.storage.set(
            &[b"block", block.digest.as_ref()].concat(),
            serialized.as_bytes(),
        );
    }

    fn latest_block(&self) -> Option<Block> {
        let latest_hash = self.storage.get(b"latest_block")?;
        self.block_by_hash(&latest_hash)
    }

    fn block_by_hash(&self, hash: &[u8]) -> Option<Block> {
        let bytes = self.storage.get(&[b"block", hash].concat())?;
        serde_json::from_slice(&bytes).unwrap_or(None)
    }

    fn maybe_bootstrap(&self) {
        if self.latest_block().is_none() {
            self.insert_block(
                Block {
                    digest: [0; 32],
                    previous_digest: [0; 32],
                    recipts: vec![],
                    time: 0,
                },
                true,
            );
        }
    }
}

struct BlockBuilder {
    transactions: Vec<ContractRecipt>,
}

impl BlockBuilder {
    fn new() -> Self {
        Self {
            transactions: vec![],
        }
    }

    fn with_transactions(transactions: Vec<ContractRecipt>) -> Self {
        Self { transactions }
    }

    fn tx(&mut self, tx: ContractRecipt) {
        self.transactions.push(tx);
    }

    fn build(self, previous_digest: [u8; 32]) -> Block {
        let time = Utc::now().timestamp_millis();
        let buf = &mut [0; 32];
        hash_recipts(&self.transactions, time, buf);
        Block {
            digest: *buf,
            previous_digest,
            recipts: self.transactions,
            time,
        }
    }
}

pub struct Chain {
    storage: BlockStorage,
    finalized_block: Block,
}

impl Chain {
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        let storage = BlockStorage::new(storage);
        storage.maybe_bootstrap();

        let finalized_block = storage
            .latest_block()
            .expect("Could not bootstrap the chain");
        Self {
            storage,
            finalized_block,
        }
    }

    pub fn insert_block(&self, block: Block) {
        self.storage.insert_block(block, true);
    }

    pub fn block_with_transactions(&self, transactions: Vec<ContractRecipt>) -> Block {
        BlockBuilder::with_transactions(transactions).build(self.finalized_block.digest)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::storage::{RocksdbStorage, Storage};

    use super::{Chain, ContractRecipt};
    use serde_json::json;
    use serial_test::serial;

    fn setup_chain() -> Chain {
        let config = Default::default();
        let storage: Arc<dyn Storage> = RocksdbStorage::load(&config);
        Chain::new(storage)
    }

    #[test]
    #[serial]
    fn insert_new_block() {
        let chain = setup_chain();
        let block = chain.block_with_transactions(vec![ContractRecipt {
            contract_name: String::from("ginger"),
            contract_method: String::from("transfer"),
            req: json!({ "from": "ginger", "to": "hello", "amount": 100_u64 }),
        }]);
    }
}
