use derive_more::{Display, Error};
use serde_derive::Deserialize;

#[derive(Debug, Deserialize, Error, Display)]
pub enum Error {
    SchemaError,
    GetError,
}
