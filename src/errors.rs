use serde_derive::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize, Error)]
pub enum Error {
    #[error("schema is invalid")]
    Schema,
    #[error("a get operation failed")]
    Get,
    #[error("an irrecoverable error was occured in the contract executer")]
    ContractIrrecoverable,
    #[error("a recoverable error was occured in the contract executer")]
    ContractRecoverable,
}
