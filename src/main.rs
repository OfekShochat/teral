use primitive_types::U256;

use crate::{
    config::TeralConfig,
    validator::Validator,
};

mod chain;
mod config;
mod contracts;
mod p2p;
mod storage;
mod validator;

fn main() {
    tracing_subscriber::fmt()
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_max_level(tracing::Level::DEBUG)
        .compact()
        .init();
    let config = TeralConfig::read("teral.toml");
    let mut validator = Validator::new(config);

    contracts::poop().unwrap();
    // TODO: how are we gonna verify a request is valid? we can make `from` a standard key that we
    // insert.
}
