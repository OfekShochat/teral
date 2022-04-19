use {
    self::native::execute_native,
    crate::storage::Storage,
    rhai::{serde::to_dynamic, Dynamic, Engine, Map, Scope, AST},
    serde_json::Value,
    std::{
        collections::{HashMap, HashSet},
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{channel, Receiver},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::Duration,
    },
    thiserror::Error,
};

pub(crate) mod language;
mod compiler;
mod native;

pub use language::execute;
pub use compiler::parse;

pub fn native_init(storage: Arc<dyn Storage>) {
    native::teral_init(ContractStorage::new(storage));
}

const CONTRACT_QUEUE_SIZE: usize = 1024;
const SYNC_RESPONDER_TIMEOUT: Duration = Duration::from_millis(100);

use rhai::EvalAltResult;
use serde_json::to_string;

use self::native::teral_transfer;

#[derive(Debug, Error)]
pub enum ContractsError {
    #[error("Schema is invalid")]
    Schema,
    #[error("a get operation failed")]
    Get,
    #[error("Could not convert from utf8")]
    FromUtf8Error(#[from] std::string::FromUtf8Error),
    #[error("Could not find native contract {0}")]
    NonExistingNative(String),
}

fn validate_schema(schema: &str, req: &Value) -> Result<(), ContractsError> {
    // schema example: "from:str;to:str;amount:i64"
    let values = schema.split(';');
    for v in values {
        let (name, typ) = v.split_once(':').ok_or(ContractsError::Schema)?;
        let value = req.get(name).ok_or(ContractsError::Schema)?;

        let is_ok = match typ {
            "i64" => value.is_i64(),
            "u64" => value.is_u64(),
            "str" => value.is_string(),
            _ => false,
        };
        if !is_ok {
            return Err(ContractsError::Schema);
        }
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct ContractStorage {
    storage: Arc<dyn Storage>,
    curr_contract: String,
    contracts_to_execute: Vec<String>,
}

unsafe impl Send for ContractStorage {}

impl ContractStorage {
    fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            curr_contract: String::from(""),
            contracts_to_execute: vec![],
        }
    }

    fn set_curr_contract(&mut self, name: &str) {
        self.contracts_to_execute = vec![];
        self.curr_contract = name.to_string();
    }

    fn regular_set_segment(&mut self, key: &str, value: Map) {
        self.storage.set(
            &[self.curr_contract.as_bytes(), key.as_bytes()].concat(),
            format!("{:?}", value).as_bytes(),
        );
    }

    fn regular_get_segment(&mut self, key: &str) -> Dynamic {
        let g = self
            .storage
            .get(&[self.curr_contract.as_bytes(), key.as_bytes()].concat());
        match g {
            Some(g) => to_dynamic::<Dynamic>(serde_json::from_slice(&g).unwrap_or_default())
                .unwrap_or_default(),
            None => Dynamic::ZERO,
        }
    }

    fn native_transfer(&mut self, to: &str, amount: u64) -> Result<(), Box<EvalAltResult>> {
        teral_transfer(
            &self,
            &serde_json::json!({ "from": self.curr_contract, "to": to, "amount": amount }),
        )
        .map_err(|_| EvalAltResult::ErrorFor(rhai::Position::new(1, 1)))?;
        // TODO: somehow execute the contract now instead of later.
        if self.get_author(&self.curr_contract).is_ok() {
            self.contracts_to_execute.push(to.to_string());
        } else {
            return Err(Box::new(EvalAltResult::ErrorFor(rhai::Position::new(1, 1))));
        }
        Ok(())
    }

    fn native_get_segment(&self, key: &str) -> Option<Value> {
        let g = self.storage.get(&[b"native", key.as_bytes()].concat())?;
        serde_json::from_slice(&g).unwrap_or_default()
    }

    fn native_set_segment(&self, key: &str, value: Value) {
        self.storage.set(
            &[b"native", key.as_bytes()].concat(),
            to_string(&value).unwrap_or_default().as_bytes(),
        );
    }

    fn add_contract(&self, name: &str, code: &str, schema: &str, author: [u8; 32]) {
        let entrypoint_key = [name.as_bytes(), b"entrypoint"].concat();
        let schema_key = [name.as_bytes(), b"schema"].concat();
        let author_key = [name.as_bytes(), b"author"].concat();

        self.storage.set(&entrypoint_key, code.as_bytes());
        self.storage.set(&schema_key, schema.as_bytes());
        self.storage.set(&author_key, &author);
    }

    fn get_code(&self, name: &str) -> Result<String, ContractsError> {
        let key = [name.as_bytes(), b"entrypoint"].concat();
        Ok(String::from_utf8(
            self.storage.get(&key).ok_or(ContractsError::Get)?,
        )?)
    }

    fn get_schema(&self, name: &str) -> Result<String, ContractsError> {
        let key = [name.as_bytes(), b"schema"].concat();
        Ok(String::from_utf8(
            self.storage.get(&key).ok_or(ContractsError::Get)?,
        )?)
    }

    fn get_author(&self, name: &str) -> Result<Vec<u8>, ContractsError> {
        let key = [name.as_bytes(), b"author"].concat();
        self.storage.get(&key).ok_or(ContractsError::Get)
    }
}

#[derive(Debug, Clone)]
pub struct ContractRequest {
    author: [u8; 32], // provided already verified
    pub name: String,
    pub method_name: String,
    pub req: Value,
    id: usize,
}

impl ContractRequest {
    pub fn new(author: [u8; 32], name: String, method_name: String, req: Value, id: usize) -> Self {
        Self {
            author,
            name,
            method_name,
            req,
            id,
        }
    }
}

#[derive(Debug)]
struct ContractResponse {
    id: usize,
    ok: bool,
}

struct ContractQueue(Mutex<HashMap<String, Mutex<Vec<ContractRequest>>>>);

impl ContractQueue {
    fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }

    fn get_and_maybe_delete(&self) -> Option<ContractRequest> {
        let mut locked_queue = self.0.lock().unwrap();
        // NOTE: this may be simplified with drain_filter: https://doc.rust-lang.org/beta/unstable-book/library-features/drain-filter.html
        for (name, lock) in locked_queue.iter() {
            let to_return = if let Ok(mut v) = lock.try_lock() {
                let to_return = v.pop();
                Some((to_return, v.is_empty()))
            } else {
                None
            };
            if let Some((to_return, remove)) = to_return {
                if remove {
                    let name = name.clone();
                    locked_queue.remove(&name);
                }
                return to_return;
            }
        }
        None
    }

    fn add(&self, req: ContractRequest) {
        let mut locked_queue = self.0.lock().unwrap();
        if locked_queue.contains_key(&req.name) {
            locked_queue
                .get(&req.name)
                .unwrap()
                .lock()
                .unwrap()
                .push(req);
        } else {
            locked_queue.insert(req.name.clone(), Mutex::new(vec![req]));
        }
    }
}

pub struct ContractExecuter {
    handlers: Vec<JoinHandle<()>>,
    queue: Arc<ContractQueue>,
    responder: Receiver<ContractResponse>,

    curr_id: usize,
    valid: Vec<ContractRequest>,
}

impl ContractExecuter {
    pub fn new(storage: Arc<dyn Storage>, exit: Arc<AtomicBool>, thread_number: usize) -> Self {
        assert!(thread_number > 0);

        let storage = ContractStorage::new(storage);

        let queue = Arc::new(ContractQueue::new());

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
                        engine.set_max_expr_depths(32, 32);
                        engine.register_type::<ContractStorage>();
                        engine.register_fn("get", ContractStorage::regular_get_segment);
                        engine.register_fn("set", ContractStorage::regular_set_segment);
                        engine.register_result_fn(
                            "native_transfer",
                            ContractStorage::native_transfer,
                        );
                        engine.on_print(|_| {});

                        let scope = &mut Scope::new();
                        loop {
                            if exit.load(Ordering::Relaxed) {
                                break;
                            }

                            if let Some(mut job) = queue.get_and_maybe_delete() {
                                job.req["from"] = Value::String(base64::encode(job.author));

                                let ok = Self::executer_thread(
                                    &mut storage,
                                    &mut cache,
                                    scope,
                                    &engine,
                                    job.clone(),
                                )
                                .is_ok();
                                sender.send(ContractResponse { id: job.id, ok }).unwrap();
                                scope.clear();
                            }
                        }
                    })
                    .unwrap()
            })
            .collect();
        tracing::info!("contracts executer(s) running.");
        Self {
            handlers,
            queue,
            responder: receiver,
            curr_id: 0,
            valid: vec![],
        }
    }

    fn executer_thread(
        storage: &mut ContractStorage,
        cache: &mut HashMap<String, AST>,
        scope: &mut Scope,
        engine: &Engine,
        job: ContractRequest,
    ) -> Result<(), ()> {
        match job.name.as_str() {
            "native" => execute_native(&job, cache, engine, storage)?,
            _ => {
                if let Ok(schema) = storage.get_schema(&job.name) {
                    if validate_schema(&schema, &job.req).is_err() {
                        return Err(());
                    }
                } else {
                    return Err(());
                }

                storage.set_curr_contract(&job.name);
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

    #[deprecated]
    pub fn execute_multiple(&self, requests: &[ContractRequest]) -> Vec<ContractRequest> {
        let mut out = Vec::with_capacity(requests.len());
        let mut enqueued = HashSet::new();

        // TODO: time limit here?
        let mut i = 0;
        let mut received_recipts = 0;
        loop {
            if !enqueued.contains(&requests[i].name) {
                enqueued.insert(&requests[i].name);
                self.queue.add(requests[i].clone());
                i += 1;
            }
            if let Ok(recipt) = self.responder.recv_timeout(SYNC_RESPONDER_TIMEOUT) {
                println!("{:?}", recipt);
                received_recipts += 1;
                enqueued.remove(&requests[recipt.id].name);
                if recipt.ok {
                    out.push(requests[recipt.id].clone()); // so many clones...
                }
                if received_recipts == requests.len() {
                    return out;
                }
            }
        }
    }

    pub fn schedule(&mut self, mut request: ContractRequest) {
        request.id = self.curr_id;
        self.curr_id += 1;
        self.valid.push(request.clone());
        self.queue.add(request);
    }

    pub fn summary(&mut self) -> &[ContractRequest] {
        for _ in 0..self.curr_id {
            if let Ok(response) = self.responder.recv_timeout(SYNC_RESPONDER_TIMEOUT) {
                if !response.ok {
                    self.valid.remove(response.id);
                }
            }
        }
        &self.valid
    }

    pub fn join(self) {
        for h in self.handlers {
            h.join().unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{atomic::AtomicBool, Arc};

    use crate::storage::{RocksdbStorage, Storage};
    use serial_test::serial;

    #[test]
    #[serial]
    fn execute_sync() {
        let exit = Arc::new(AtomicBool::new(false));

        let config = Default::default();
        let storage: Arc<dyn Storage> = RocksdbStorage::load(&config);
        let executer = super::ContractExecuter::new(storage.clone(), exit.clone(), 1);
        let recipts = executer.execute_multiple(&[
            super::ContractRequest::new(
                [0; 32],
                String::from("native"),
                String::from("add"),
                serde_json::json!({ "name": "test-sync", "code": r#"
fn transfer(req) {
    storage.set(req["from"], #{ "balance": 1000 });
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
            super::ContractRequest::new(
                [0; 32],
                String::from("test-sync"),
                String::from("transfer"),
                serde_json::json!({"from": "hello", "to": "ginger", "amount": 100_u64}),
                1,
            ),
        ]);
        exit.store(true, std::sync::atomic::Ordering::SeqCst);
        executer.join();
        storage.delete_prefix("test-test".as_bytes());

        assert!(recipts.len() == 2);
    }

    #[test]
    #[serial]
    fn execute_async() {
        let exit = Arc::new(AtomicBool::new(false));

        let config = Default::default();
        let storage: Arc<dyn Storage> = RocksdbStorage::load(&config);
        let mut executer = super::ContractExecuter::new(storage.clone(), exit.clone(), 1);
        executer.schedule(super::ContractRequest::new(
            [0; 32],
            String::from("native"),
            String::from("add"),
            serde_json::json!({ "name": "test-async", "code": r#"
fn transfer(req) {
    storage.set(req["from"], #{ "balance": 1000 });
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
        ));
        executer.schedule(super::ContractRequest::new(
            [0; 32],
            String::from("test-async"),
            String::from("transfer"),
            serde_json::json!({"from": "hello", "to": "ginger", "amount": 100_u64}),
            1,
        ));
        exit.store(true, std::sync::atomic::Ordering::SeqCst);
        storage.delete_prefix("test-test".as_bytes());

        println!("{:?}", executer.summary());

        // assert!(executer.summary().len() == 2);
        executer.join();
    }
}
