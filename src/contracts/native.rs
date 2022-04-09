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
            // TODO: maybe call here script.init() so the code can init its storage (for example give
            // the initial supply).
            Ok(())
        }
        "transfer" => teral_transfer(storage, &job.req),
        "stake" => teral_stake(storage, &job.req),
        _ => Err(()),
    }
}

pub(crate) fn teral_transfer(storage: &ContractStorage, req: &Value) -> Result<(), ()> {
    let from = storage.native_get_segment(req["from"].as_str().unwrap());
    let from = if let Some(from) = from {
        from
    } else {
        return Err(());
    };
    if req["amount"].as_u64().unwrap() > from["balance"].as_u64().unwrap() {
        return Err(());
    }

    storage.native_set_segment(
        req["from"].as_str().unwrap(),
        json!({ "balance": from["balance"].as_u64().unwrap() - req["amount"].as_u64().unwrap() }),
    );

    let to = storage.native_get_segment(req["to"].as_str().unwrap());

    if let Some(to) = to {
        let balance = to["balance"].as_u64().unwrap() + req["amount"].as_u64().unwrap();
        storage.native_set_segment(req["to"].as_str().unwrap(), json!({ "balance": balance }));
    } else {
        // if req["to"].as_str().unwrap().len() != 32 {
        //     return Err(()); // names with 32 characters are not contract names (most probably), and if we dont have it then no reason to waste money.
        // }
        storage.native_set_segment(
            req["to"].as_str().unwrap(),
            json!({ "balance": req["amount"].as_u64().unwrap() }),
        );
    }
    Ok(())
}

pub(crate) fn teral_stake(storage: &ContractStorage, req: &Value) -> Result<(), ()> {
    Ok(())
}

pub(crate) fn teral_init(storage: ContractStorage) {
    storage.native_set_segment("ghostway", json!({ "balance": 1000_u64 }));
}
