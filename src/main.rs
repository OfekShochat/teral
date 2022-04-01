use std::sync::{atomic::AtomicBool, Arc};

use crate::storage::Storage;

mod chain;
mod config;
mod contracts;
pub mod errors;
mod p2p;
mod storage;
mod validator;

fn main() {
    println!("Hello, world!");
    let config = config::config_from_file("teral.toml");
    let storage: Arc<dyn Storage> = storage::RocksdbStorage::load("db/");
    let exit = Arc::new(AtomicBool::new(false));

    let executer = contracts::ContractExecuter::new(storage, exit, 3);
    let a = executer.execute_multiple(&[contracts::ContractRequest::new([0; 32], String::from("hello"), String::from("transfer"), serde_json::json!({ "from": "ginger", "to": "ofek", "amount": 100_u64 }), 16)]);
    println!("{:?}", a);
}
