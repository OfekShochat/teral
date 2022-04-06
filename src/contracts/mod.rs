use {
    crate::storage::Storage,
    rhai::{serde::to_dynamic, Dynamic, Engine, Map, Scope, AST},
    serde_json::Value,
    std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc::{channel, Receiver},
        },
    },
    std::{
        sync::{Arc, Mutex},
        thread::{self, JoinHandle},
    },
    thiserror::Error,
};

const CONTRACT_QUEUE_SIZE: usize = 1024;

#[derive(Debug, Error)]
pub enum ContractsError {
    #[error("Schema is invalid")]
    Schema,
    #[error("a get operation failed")]
    Get,
    #[error("Could not convert from utf8")]
    FromUtf8Error(#[from] std::string::FromUtf8Error),
}

fn validate_schema(schema: &str, req: &Value) -> Result<(), ContractsError> {
    // schema example: "from:str;to:str;amount:i64"
    let values = schema.split(';');
    for v in values {
        let (name, typ) = v.split_once(';').ok_or(ContractsError::Schema)?;
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

    fn set_segment(&mut self, key: &str, value: Map) {
        self.storage.set(
            &[self.curr_contract.as_bytes(), key.as_bytes()].concat(),
            format!("{:?}", value).as_bytes(),
        );
    }

    fn get_segment(&mut self, key: &str) -> Dynamic {
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
        Ok(self.storage.get(&key).ok_or(ContractsError::Get)?)
    }
}

#[derive(Debug, Clone)]
pub struct ContractRequest {
    author: [u8; 32], // provided already verified
    name: String,
    method_name: String,
    req: Value,
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

pub struct ContractExecuter {
    handlers: Vec<JoinHandle<()>>,
    queue: Arc<Mutex<Vec<ContractRequest>>>,
    responder: Receiver<ContractResponse>,
}

impl ContractExecuter {
    pub fn new(storage: Arc<dyn Storage>, exit: Arc<AtomicBool>, thread_number: usize) -> Self {
        assert!(thread_number > 0);

        let storage = ContractStorage::new(storage);

        let queue = Arc::new(Mutex::new(Vec::<ContractRequest>::with_capacity(
            // rust for some reason can't infere the type of this vec when cloning @181.
            CONTRACT_QUEUE_SIZE,
        )));

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
                                break;
                            }
                            if let Some(job) = queue.lock().unwrap().pop() {
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

        tracing::debug!("contracts executer running.");
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
        match job.name.as_str() {
            "native" if job.method_name == "add" => {
                if let Ok(original_author) = storage.get_author(&job.name) {
                    if job.author.to_vec() != original_author
                        || validate_schema("name:str;code:str;schema:str", &job.req).is_err()
                    {
                        return Err(());
                    }
                }

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
            }
            _ => {
                if let Ok(schema) = storage.get_schema(&job.name) {
                    if validate_schema(&schema, &job.req).is_err() {
                        return Err(());
                    }
                } else {
                    return Err(());
                }

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

        {
            let mut locked_queue = self.queue.lock().unwrap();
            requests.iter().for_each(|r| locked_queue.push(r.clone()));
        }

        // TODO: time limit here?
        for _ in 0..requests.len() {
            let recipt = self.responder.recv().unwrap();
            println!("{:?}", recipt);
            if recipt.ok {
                out.push(requests[recipt.id].clone()); // so many clones...
            }
        }
        out
    }

    pub fn join(self) {
        for h in self.handlers {
            h.join().unwrap();
        }
    }
}
