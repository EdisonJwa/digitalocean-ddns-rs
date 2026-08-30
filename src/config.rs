use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

const DEFAULT_TTL: u64 = 60;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
#[allow(clippy::upper_case_acronyms)]
pub enum RecordType {
    A,
    AAAA,
}

impl RecordType {
    pub fn is_v4(self) -> bool {
        self == RecordType::A
    }
}

impl std::fmt::Display for RecordType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RecordType::A => "A",
            RecordType::AAAA => "AAAA",
        })
    }
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub token: String,
    pub records: HashMap<String, RecordConfig>,
}

#[derive(Deserialize, Debug)]
pub struct RecordConfig {
    pub domain: String,
    pub name: String,
    #[serde(rename = "type")]
    pub type_: RecordType,
    #[serde(default = "default_ttl")]
    pub ttl: u64,
    pub interface: Option<String>,
    #[serde(default)]
    pub use_cn_dns: bool,
}

fn default_ttl() -> u64 {
    DEFAULT_TTL
}

pub fn load(path: &str) -> Result<Config> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read config file '{path}'"))?;
    let config: Config =
        toml::from_str(&text).with_context(|| format!("failed to parse config file '{path}'"))?;
    if config.records.is_empty() {
        anyhow::bail!("no records defined in config file '{path}'");
    }
    Ok(config)
}
