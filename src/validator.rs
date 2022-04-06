use std::sync::Arc;

use crate::{config::TeralConfig, chain::Chain};

pub struct Validator {
    // schedule: LeaderScheduler,
    chain: Arc<Chain>, // arc to share between here and the rpc service.
}

impl Validator {
    pub fn new(config: TeralConfig) -> Self {
        let storage = config.load_storage();
        let chain = Arc::new(Chain::new(storage.unwrap()));
        Self { chain }
    }
}
