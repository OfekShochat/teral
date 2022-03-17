use rhai::{serde::to_dynamic, Dynamic, Map, Scope};

use {
    crate::{errors::Error, storage::Storage},
    rhai::Engine,
    serde_json::Value,
    std::{
        sync::{Arc, Mutex},
        thread::{self, JoinHandle},
    },
};

const CONTRACT_QUEUE_SIZE: usize = 1024;

fn validate_schema(schema: &str, req: &Value) -> anyhow::Result<()> {
    // schema example: "from:str;to:str;amount:i64"
    let values = schema.split(";");
    for v in values {
        let (name, typ) = v.split_once(":").ok_or(Error::SchemaError)?;
        let value = req.get(name).ok_or(Error::SchemaError)?;

        let is_ok = match typ {
            "i64" => value.is_i64(),
            "u64" => value.is_u64(),
            "str" => value.is_string(),
            _ => false,
        };
        if !is_ok {
            return Err(Error::SchemaError.into());
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ContractStorage {
    storage: Arc<dyn Storage>,
    curr_contract: String,
}

unsafe impl Send for ContractStorage {}

impl ContractStorage {
    fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            curr_contract: String::from(""),
        }
    }

    fn set_curr_contract(&mut self, name: &str) {
        self.curr_contract = name.to_string();
    }

    fn set_segment(&self, key: &str, value: Map) {
        self.storage.set(
            &[self.curr_contract.as_bytes(), key.as_bytes()].concat(),
            format!("{:?}", value).as_bytes(),
        );
    }

    fn get_segment(&self, key: &str) -> Dynamic {
        let g = self
            .storage
            .get(&[self.curr_contract.as_bytes(), key.as_bytes()].concat());
        match g {
            Some(g) => to_dynamic::<Dynamic>(serde_json::from_slice(&g).unwrap_or_default())
                .unwrap_or_default(),
            None => Dynamic::ZERO,
        }
    }

    fn add_contract(&self, name: &str, code: &str, schema: &str) {
        let entrypoint_key = [name.as_bytes(), b"entrypoint"].concat();
        let schema_key = [name.as_bytes(), b"schema"].concat();

        self.storage.set(&entrypoint_key, code.as_bytes());
        self.storage.set(&schema_key, schema.as_bytes());
    }

    fn get_code(&self, name: &str) -> anyhow::Result<String> {
        let key = [name.as_bytes(), b"entrypoint"].concat();
        Ok(String::from_utf8(
            self.storage.get(&key).ok_or(Error::GetError)?,
        )?)
    }

    fn get_schema(&self, name: &str) -> anyhow::Result<String> {
        let key = [name.as_bytes(), b"schema"].concat();
        Ok(String::from_utf8(
            self.storage.get(&key).ok_or(Error::GetError)?,
        )?)
    }
}

struct ContractRequest {
    name: String,
    method_name: String,
    req: Value,
}

struct ContractExecuter {
    handlers: Vec<JoinHandle<()>>,
    queue: Arc<Mutex<Vec<ContractRequest>>>,
}

impl ContractExecuter {
    pub fn new(storage: Arc<dyn Storage>, thread_number: usize) -> Self {
        assert!(thread_number > 0);

        let storage = ContractStorage::new(storage);

        let queue = Arc::new(Mutex::new(Vec::with_capacity(CONTRACT_QUEUE_SIZE)));
        let handlers = (0..thread_number)
            .map(|i| {
                let queue = queue.clone();
                let storage = storage.clone();
                thread::Builder::new()
                    .name(format!("contract-worker({})", i))
                    .spawn(move || {
                        Self::executer_thread(queue, storage);
                    })
                    .unwrap()
            })
            .collect();

        Self { handlers, queue }
    }

    fn executer_thread(queue: Arc<Mutex<Vec<ContractRequest>>>, mut storage: ContractStorage) {
        let mut engine = Engine::new();
        engine.register_type::<ContractStorage>();
        engine.register_fn("get", ContractStorage::get_segment);
        engine.register_fn("set", ContractStorage::set_segment);
        engine.on_print(|_| {});

        loop {
            if let Some(job) = queue.lock().unwrap().pop() {
                match job.name.as_str() {
                    "native"
                        if job.method_name == "add"
                            && validate_schema("name:str;code:str;schema:str", &job.req)
                                .is_ok() =>
                    {
                        storage.add_contract(
                            &job.req["name"].as_str().unwrap(),
                            &job.req["code"].as_str().unwrap(),
                            &job.req["schema"].as_str().unwrap(),
                        );
                    }
                    _ => {
                        storage.set_curr_contract(&job.method_name);
                        let schema = match storage.get_schema(&job.name) {
                            Ok(schema) => schema,
                            Err(_) => continue,
                        };

                        // is this necessary? we already verify when adding.
                        if validate_schema(&schema, &job.req).is_err() {
                            continue; // TODO: should report back to the main thread so that we don't take this contract again.
                        }

                        let code = match storage.get_code(&job.name) {
                            Ok(code) => code,
                            Err(_) => continue,
                        };

                        let ast = match engine.compile(code) {
                            Ok(ast) => ast,
                            Err(_) => continue, // should report back to the main thread so that we don't take this contract again.
                        };

                        let scope = &mut Scope::new();
                        scope.push_constant("storage", storage.clone());

                        let req_arg = match to_dynamic(job.req) {
                            Ok(args) => args,
                            Err(_) => continue,
                        };

                        let _ = engine
                            .call_fn_raw(
                                scope,
                                &ast,
                                false,
                                false,
                                job.method_name,
                                None,
                                &mut [req_arg],
                            )
                            .unwrap();
                    }
                }
            }
        }
    }
}
