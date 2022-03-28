mod config;
mod contracts;
pub mod errors;
mod p2p;
mod storage;
mod validator;

fn main() {
    println!("Hello, world!");
    let config = config::config_from_file("teral.toml");
}
