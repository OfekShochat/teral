use crate::config::{TeralConfig, config_from_file};

pub struct Validator {
    config: TeralConfig,
}

impl Validator {
    pub fn new(config_path: &str) -> Self {
        let config = config_from_file(config_path);
        Self { config }
    }
}
