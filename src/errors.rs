use serde_derive::Deserialize;
use derive_more::{Error, Display};

#[derive(Debug, Deserialize, Error, Display)]
pub enum Error {
    SchemaError,
    GetError,
}
