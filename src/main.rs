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

    let executer = contracts::ContractExecuter::new(storage, exit.clone(), 3);
    let a = executer.execute_multiple(&[
        contracts::ContractRequest::new([0; 32],
        String::from("native"), String::from("add"),
        serde_json::json!({ "name": "ginger", "code": r#"
fn transfer(req) {
    let from = storage.get(req["from"]);
    if from == 0 || from["balance"] < req["amount"] { return; }
    from["balance"] -= req["amount"];
    storage.set(req["from"], from);
    let to = storage.get(req["to"]);
    if to == 0 {
        storage.set(req["to"], #{ "balance": req["amount"] })
    } else {
        to["balance"] += req["amount"];
        storage.set(req["to"], to);
    }
}
"#, "schema": "from:str;to:str;amount:u64" }), 0)
    ]);
    exit.store(true, std::sync::atomic::Ordering::SeqCst);
    println!("{:?}", a);
}
