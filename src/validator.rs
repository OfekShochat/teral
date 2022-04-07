use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use crate::{
    chain::{requests_to_recipts, Block, Chain},
    config::TeralConfig,
    contracts::{ContractExecuter, ContractRequest},
};

pub struct Validator {
    // schedule: LeaderScheduler,
    exit: Arc<AtomicBool>,
    chain: Arc<Chain>, // arc to share between here and the rpc service.
    contract_executer: ContractExecuter,
}

impl Validator {
    pub fn new(config: TeralConfig) -> Self {
        let exit = Arc::new(AtomicBool::new(false));

        let storage = config.load_storage().unwrap();
        let chain = Arc::new(Chain::new(storage.clone()));
        let contract_executer = ContractExecuter::new(storage, exit.clone(), config.contracts_exec.threads);
        Self {
            exit,
            chain,
            contract_executer,
        }
    }

    pub fn schedule_contract(&mut self, req: ContractRequest) {
        self.contract_executer.schedule(req);
    }

    pub fn finalize_block(&mut self) {
        let block = self.finalize_contracts();
        self.chain.insert_block(block);
    }

    pub fn finalize_contracts(&mut self) -> Block {
        let transactions = self.contract_executer.summary();
        self.chain
            .block_with_transactions(requests_to_recipts(transactions.to_vec()))
    }

    pub fn stop(self) {
        self.exit.store(true, Ordering::SeqCst);
        self.contract_executer.join();
    }
}
