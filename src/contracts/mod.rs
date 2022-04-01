use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{channel, Receiver},
    },
};

use rhai::AST;

use {
    crate::{errors::Error, storage::Storage},
    rhai::{serde::to_dynamic, Dynamic, Engine, Map, Scope},
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
        let (name, typ) = v.split_once(":").ok_or(Error::Schema)?;
        let value = req.get(name).ok_or(Error::Schema)?;

        let is_ok = match typ {
            "i64" => value.is_i64(),
            "u64" => value.is_u64(),
            "str" => value.is_string(),
            _ => false,
        };
        if !is_ok {
            return Err(Error::Schema.into());
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

    fn add_contract(&self, name: &str, code: &str, schema: &str, author: [u8; 32]) {
        let entrypoint_key = [name.as_bytes(), b"entrypoint"].concat();
        let schema_key = [name.as_bytes(), b"schema"].concat();
        let author_key = [name.as_bytes(), b"author"].concat();

        self.storage.set(&entrypoint_key, code.as_bytes());
        self.storage.set(&schema_key, schema.as_bytes());
        self.storage.set(&author_key, &author);
    }

    fn get_code(&self, name: &str) -> anyhow::Result<String> {
        let key = [name.as_bytes(), b"entrypoint"].concat();
        Ok(String::from_utf8(
            self.storage.get(&key).ok_or(Error::Get)?,
        )?)
    }

    fn get_schema(&self, name: &str) -> anyhow::Result<String> {
        let key = [name.as_bytes(), b"schema"].concat();
        Ok(String::from_utf8(
            self.storage.get(&key).ok_or(Error::Get)?,
        )?)
    }

    fn get_author(&self, name: &str) -> anyhow::Result<Vec<u8>> {
        let key = [name.as_bytes(), b"author"].concat();
        Ok(self.storage.get(&key).ok_or(Error::Get)?)
    }
}

#[derive(Clone)]
struct ContractRequest {
    author: [u8; 32], // provided already verified
    name: String,
    method_name: String,
    req: Value,
    id: usize,
}

struct ContractExecuter {
    handlers: Vec<JoinHandle<()>>,
    queue: Arc<Mutex<Vec<ContractRequest>>>,
    responder: Receiver<ContractResponse>,
}

struct ContractResponse {
    id: usize,
    ok: bool,
}

impl ContractExecuter {
    pub fn new(storage: Arc<dyn Storage>, exit: Arc<AtomicBool>, thread_number: usize) -> Self {
        assert!(thread_number > 0);

        let storage = ContractStorage::new(storage);

        let queue = Arc::new(Mutex::new(Vec::<ContractRequest>::with_capacity(CONTRACT_QUEUE_SIZE))); // if we resolve @177 we dont need this generic

        let (sender, receiver) = channel();

        let handlers = (0..thread_number)
            .map(|i| {
                let queue = queue.clone();
                let mut storage = storage.clone();
                let exit = exit.clone();
                let sender = sender.clone();
                thread::Builder::new()
                    .name(format!("contract-worker({})", i))
                    .spawn(move || {
                        let mut cache = HashMap::new();

                        let mut engine = Engine::new();
                        engine.register_type::<ContractStorage>();
                        engine.register_fn("get", ContractStorage::get_segment);
                        engine.register_fn("set", ContractStorage::set_segment);
                        engine.on_print(|_| {});

                        let scope = &mut Scope::new();
                        loop {
                            if exit.load(Ordering::Relaxed) {
                                if i == 0 {
                                    // save to db
                                }
                                break;
                            }
                            if let Some(job) = queue.lock().unwrap().pop() {
                                match Self::executer_thread(
                                    &mut storage,
                                    &mut cache,
                                    scope,
                                    &engine,
                                    job.clone(), // probably we dont need this clone
                                ) {
                                    Ok(()) => {
                                        sender.send(ContractResponse { id: job.id, ok: true }).unwrap()
                                    }
                                    Err(()) => {
                                        sender.send(ContractResponse { id: job.id, ok: false }).unwrap()
                                    }
                                }
                                scope.clear();
                            }
                        }
                    })
                    .unwrap()
            })
            .collect();

        Self {
            handlers,
            queue,
            responder: receiver,
        }
    }

    fn executer_thread(
        storage: &mut ContractStorage,
        cache: &mut HashMap<String, AST>,
        scope: &mut Scope,
        engine: &Engine,
        job: ContractRequest,
    ) -> Result<(), ()> {
        // do this from where we call this function.
        match job.name.as_str() {
            "native"
                if job.method_name == "add"
                    && validate_schema("name:str;code:str;schema:str", &job.req).is_ok() =>
            {
                let original_author = storage.get_author(&job.name).unwrap();
                if job.author.to_vec() != original_author {
                    return Err(());
                }

                match engine.compile(job.req["code"].as_str().unwrap()) {
                    Ok(ast) => {
                        let name = job.req["name"].as_str().unwrap().to_string();
                        cache.insert(name, ast);
                        storage.add_contract(
                            &job.req["name"].as_str().unwrap(),
                            &job.req["code"].as_str().unwrap(),
                            &job.req["schema"].as_str().unwrap(),
                            job.author,
                        );
                    }
                    Err(_) => return Err(()),
                }
            }
            _ => {
                storage.set_curr_contract(&job.method_name);
                scope.push_constant("storage", storage.clone());

                let ast = if let Some(ast) = cache.get(&job.name) {
                    ast.clone()
                } else if let Ok(code) = storage.get_code(&job.name) {
                    let ast = match engine.compile(code) {
                        Ok(ast) => ast,
                        Err(_) => return Err(()),
                    };
                    cache.insert(job.name, ast.clone());
                    ast
                } else {
                    return Err(());
                };

                let req_arg = match to_dynamic(job.req) {
                    Ok(args) => args,
                    Err(_) => return Err(()),
                };

                if engine
                    .call_fn_raw(
                        scope,
                        &ast,
                        false,
                        false,
                        job.method_name,
                        None,
                        &mut [req_arg],
                    )
                    .is_err()
                {
                    return Err(());
                }
            }
        }
        Ok(())
    }

    pub fn execute_multiple(&self, requests: &[ContractRequest]) -> Vec<ContractRequest> {
        let mut out = Vec::with_capacity(requests.len());

        let mut locked_queue = self.queue.lock().unwrap();
        requests.iter().for_each(|r| locked_queue.push(r.clone()));

        // TODO: time limit here?
        for _ in 0..requests.len() {
            let recipt = self.responder.recv().unwrap();
            if recipt.ok {
                out.push(requests[recipt.id].clone()); // so many clones...
            }
        }
        out
    }
}
