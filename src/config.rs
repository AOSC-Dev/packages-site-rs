use anyhow::Result;
use serde::Deserialize;
use std::{fs::File, io::Read, path::Path};
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    pub db: Db,
    pub global: Global,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Db {
    pub pg_conn: String,
    pub abbs: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Global {
    pub data: String,
    pub listen: String,
    pub log: String,
    pub sqlx_log: String,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Config> {
        let mut file = File::open(path)?;
        let mut toml_str = String::new();
        file.read_to_string(&mut toml_str)?;
        let config: Config = toml::from_str(&toml_str)?;
        Ok(config)
    }
}
