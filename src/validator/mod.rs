mod leader_schedule;
pub use self::leader_schedule::*;

use std::{
    net::UdpSocket,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ed25519_consensus::SigningKey;

use crate::{
    chain::{requests_to_recipts, Block, Chain},
    config::TeralConfig,
    contracts::{ContractExecuter, ContractRequest},
    p2p::{ClusterInfo, GossipService},
};

pub struct Validator {
    schedule: Arc<dyn LeaderSchedule>,
    exit: Arc<AtomicBool>,
    gossip: GossipService,
    chain: Arc<Chain>, // arc to share between here and the rpc service.
    contract_executer: ContractExecuter,
}

impl Validator {
    pub fn new(config: TeralConfig) -> Self {
        let exit = Arc::new(AtomicBool::new(false));

        let storage = config.load_storage().unwrap();
        let chain = Arc::new(Chain::new(storage.clone()));
        let contract_executer =
            ContractExecuter::new(storage.clone(), exit.clone(), config.contracts_exec.threads);
        let udp_socket = UdpSocket::bind(&config.network.addr).expect(&format!("Could not bind udp socket to {}", config.network.addr));
        let cluster_info = Arc::new(ClusterInfo::new(Arc::new(SigningKey::new(&mut rand::thread_rng())), storage));
        let (gossip, gossip_receiver) = GossipService::new(cluster_info, udp_socket, &exit.clone());
        Self {
            exit,
            chain,
            contract_executer,
            gossip,
            schedule: config.get_scheduler().unwrap(),
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
        tracing::debug!("finalizing transactions: {:?}", transactions);
        self.chain
            .block_with_transactions(requests_to_recipts(transactions.to_vec()))
    }

    pub fn stop(self) {
        self.exit.store(true, Ordering::SeqCst);
        self.contract_executer.join();
    }
}