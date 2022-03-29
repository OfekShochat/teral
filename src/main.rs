mod config;
mod contracts;
pub mod errors;
mod p2p;
mod storage;
mod validator;
mod chain;

fn main() {
    println!("Hello, world!");
    let config = config::config_from_file("teral.toml");
}
