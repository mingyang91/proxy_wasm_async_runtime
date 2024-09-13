use pow_runtime::log_level::LogLevel;
use pow_types::cidr::CIDR;
use pow_types::config::VirtualHost;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
}

impl TimeUnit {
    fn as_secs(&self) -> u64 {
        match self {
            TimeUnit::Second => 1,
            TimeUnit::Minute => 60,
            TimeUnit::Hour => 3600,
            TimeUnit::Day => 86400,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RateLimit {
    pub unit: TimeUnit,
    pub requests_per_unit: u32,
}

impl RateLimit {
    pub fn current_bucket(&self) -> u64 {
        let unit: u64 = self.unit.as_secs();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("failed to get timestamp")
            .as_secs();
        timestamp / unit
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Setting {
    pub rate_limit: RateLimit,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Config<T> {
    pub virtual_hosts: Vec<VirtualHost<T>>,
    pub whitelist: Option<Vec<CIDR>>,
    pub difficulty: u64,
    pub log_level: Option<LogLevel>,
    pub mempool_upstream_name: String,
}
