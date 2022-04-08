use std::collections::HashMap;

use rhai::{Engine, AST};
use serde_json::{json, Value};

use super::{validate_schema, ContractRequest, ContractStorage};

// TODO: maybe have the native contracts in an enum with procmacro so that we can #[schema("from:str;to:str;amount:u64")] and it will implement
// the schema validation automatically.

pub(crate) fn execute_native(
    job: &ContractRequest,
    cache: &mut HashMap<String, AST>,
    engine: &Engine,
    storage: &ContractStorage,
) -> Result<(), ()> {
    match job.method_name.as_str() {
        "add" => {
            if let Ok(original_author) = storage.get_author(&job.name) {
                if job.author.to_vec() != original_author {
                    return Err(());
                }
            }
            validate_schema("name:str;code:str;schema:str", &job.req).map_err(|_| ())?;

            match engine.compile(job.req["code"].as_str().unwrap()) {
                Ok(ast) => {
                    let name = job.req["name"].as_str().unwrap().to_string();
                    cache.insert(name, ast);
                    storage.add_contract(
                        job.req["name"].as_str().unwrap(),
                        job.req["code"].as_str().unwrap(),
                        job.req["schema"].as_str().unwrap(),
                        job.author,
                    );
                }
                Err(_) => return Err(()),
            }
            Ok(())
        }
        "transfer" => teral_transfer(storage, &job.req),
        _ => Err(()),
    }
}

pub(crate) fn teral_transfer(storage: &ContractStorage, req: &Value) -> Result<(), ()> {
    let from = storage.native_get_segment(req["from"].as_str().unwrap());
    if from.is_none()
        || req["amount"].as_u64().unwrap() > from.unwrap()["balance"].as_u64().unwrap()
    {
        return Err(());
    }

    let to = storage.native_get_segment(req["to"].as_str().unwrap());
    if let Some(to) = to {
        let balance = to["balance"].as_u64().unwrap() + req["amount"].as_u64().unwrap();
        storage.native_set_segment(req["to"].as_str().unwrap(), json!({ "balance": balance }));
    } else {
        storage.native_set_segment(
            req["to"].as_str().unwrap(),
            json!({ "balance": req["amount"].as_u64().unwrap() }),
        );
    }
    Ok(())
}

pub(crate) fn teral_init(storage: &ContractStorage) {
    storage.native_set_segment("ghostway", json!({ "balance": 10000_u64 }))
}