use std::collections::HashMap;

use pow_runtime::log_level::LogLevel;
use pow_types::{cidr::CIDR, config::VirtualHost};
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub name: String,
    pub public_key: PublicKey,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RawSetting {
    pub grants: Vec<Token>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Setting {
    pub grants: HashMap<PublicKey, String>,
}

impl From<RawSetting> for Setting {
    fn from(raw: RawSetting) -> Self {
        let mut grants = HashMap::new();
        for token in raw.grants {
            grants.insert(token.public_key, token.name);
        }
        Self { grants }
    }
}

impl<'de> Deserialize<'de> for Setting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        RawSetting::deserialize(deserializer).map(Setting::from)
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Config<T> {
    pub virtual_hosts: Vec<VirtualHost<T>>,
    pub whitelist: Option<Vec<CIDR>>,
    pub log_level: Option<LogLevel>,
}
