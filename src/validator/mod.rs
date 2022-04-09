mod leader_schedule;
pub use self::leader_schedule::*;

use {
    crate::{
        chain::{requests_to_recipts, Block, Chain},
        config::TeralConfig,
        contracts::{ContractExecuter, ContractRequest},
        p2p::{ClusterInfo, GossipService},
    },
    ed25519_consensus::SigningKey,
    std::{
        net::UdpSocket,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    },
};

pub struct Validator {
    schedule: LeaderSchedule,
    exit: Arc<AtomicBool>,
    gossip: GossipService,
    chain: Arc<Chain>, // arc to share between here and the rpc service.
    contract_executer: ContractExecuter,
}

impl Validator {
    pub fn new(config: TeralConfig) -> Self {
        let exit = Arc::new(AtomicBool::new(false));

        let storage = config.load_storage().unwrap();
        // native_init(storage.clone());
        let keypair = Arc::new(SigningKey::new(&mut rand::thread_rng()));
        let chain = Arc::new(Chain::new(storage.clone(), keypair.verification_key().to_bytes()));
        let contract_executer =
            ContractExecuter::new(storage.clone(), exit.clone(), config.contracts_exec.threads);
        let udp_socket = UdpSocket::bind(&config.network.addr)
            .unwrap_or_else(|_| panic!("Could not bind udp socket to {}", config.network.addr));
        let cluster_info = Arc::new(ClusterInfo::new(
            keypair,
            storage,
        ));
        let (gossip, gossip_receiver) = GossipService::new(cluster_info, udp_socket, &exit);
        Self {
            exit,
            chain,
            contract_executer,
            gossip,
            schedule: LeaderSchedule::new(),
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
