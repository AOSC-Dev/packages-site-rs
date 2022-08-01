use anyhow::Result;
use std::{fs::File, io::Read, path::Path};
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct Config {
    pub abbs: String,
    pub piss: String,
    pub data: String,
    pub listen: String,
    pub pg_conn: String,
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
