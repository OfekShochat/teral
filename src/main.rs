use primitive_types::U256;

use crate::{config::TeralConfig, validator::Validator, contracts::execute};

mod chain;
mod config;
mod contracts;
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
    let mut validator = Validator::new(config);

    // TODO: how are we gonna verify a request is valid? we can make `from` a standard key that we
    // insert.

    validator.schedule_contract(contracts::ContractRequest::new(
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
}"#, "schema": "from:str;to:str;amount:u64" }),
        0,
    ));

    validator.schedule_contract(contracts::ContractRequest::new(
        [0; 32],
        String::from("native"),
        String::from("transfer"),
        serde_json::json!({ "from": "ghostway", "to": "ginger", "amount": 100_u64}),
        0,
    ));

    let r = validator.finalize_contracts();
    println!("{:?} {}", r, r.recipt_count());

    validator.stop();
}
