mod config;
mod contracts;
mod p2p;
mod storage;
pub mod errors;

fn main() {
    println!("Hello, world!");
    let config = config::config_from_file("teral.toml");
}
