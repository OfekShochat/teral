use std::sync::{atomic::AtomicBool, Arc};

use crate::config::TeralConfig;

mod chain;
mod config;
mod contracts;
pub mod errors;
mod p2p;
mod storage;
mod validator;

fn main() {
    tracing_subscriber::fmt()
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_max_level(tracing::Level::DEBUG)
        .compact()
        .init();
    let config = TeralConfig::read("teral.toml");

    let exit = Arc::new(AtomicBool::new(false));

    let storage = config.load_storage().unwrap();
    let executer = contracts::ContractExecuter::new(storage, exit.clone(), 8);
    let a = executer.execute_multiple(&[
        contracts::ContractRequest::new(
            [0; 32],
            String::from("native"),
            String::from("add"),
            serde_json::json!({ "name": "ginger", "code": r#"
fn transfer(req) {
    let from = storage.get(req["from"]);
    if from == 0 || from["balance"] < req["amount"] { throw; }
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
"#, "schema": "from:str;to:str;amount:u64" }),
            0,
        ),
        contracts::ContractRequest::new(
            [0; 32],
            String::from("ginger"),
            String::from("transfer"),
            serde_json::json!({"from": "hello", "to": "ginger", "amount": 100_u64}),
            1,
        ),
    ]);
    exit.store(true, std::sync::atomic::Ordering::SeqCst);
    executer.join();
    println!("{:?}", a);
}
