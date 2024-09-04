use std::{net::IpAddr, ops::Deref, str::FromStr};

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{radix_tree::{Matches, RadixTree}, trie::Trie, RouteError};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VirtualHost<T> {
    pub host: String,
    pub routes: Vec<Route<T>>,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Route<T> {
    pub path: String,
    #[serde(flatten)]
    pub config: T,
    pub children: Option<Vec<Route<T>>>,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Config<T> {
    pub virtual_hosts: Vec<VirtualHost<T>>,
    pub whitelist: Option<Vec<CIDR>>,
    pub difficulty: u64,
    pub log_level: Option<LogLevel>,
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all="snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl From<LogLevel> for proxy_wasm::types::LogLevel {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Trace => proxy_wasm::types::LogLevel::Trace,
            LogLevel::Debug => proxy_wasm::types::LogLevel::Debug,
            LogLevel::Info => proxy_wasm::types::LogLevel::Info,
            LogLevel::Warn => proxy_wasm::types::LogLevel::Warn,
            LogLevel::Error => proxy_wasm::types::LogLevel::Error,
            LogLevel::Critical => proxy_wasm::types::LogLevel::Critical,
        }
    }
}

impl <T> TryFrom<Config<T>> for Router<T> {
    type Error = RouteError;
    
    fn try_from(value: Config<T>) -> Result<Self, Self::Error> {
        let mut trie = Trie::default();
        for virtual_host in value.virtual_hosts {
            let mut radix = RadixTree::default();
            for route in virtual_host.routes {
                radix_add_all(&mut radix, &route.path, route.config, route.children)?;
            }
            trie.add(&virtual_host.host, radix)?;
        }
        Ok(Router(trie))
    }
}

fn radix_add_all<T>(radix: &mut RadixTree<T>, path: &str, config: T, children: Option<Vec<Route<T>>>) -> Result<(), RouteError> {
    radix.add(path, config)?;
    let Some(children) = children else {
        return Ok(())
    };

    for child in children {
        let path = normalize_path(&format!("{}/{}", path, child.path));
        radix_add_all(radix, &path, child.config, child.children)?;
    }
    return Ok(())
}


fn normalize_path(path: &str) -> String {
    let re = Regex::new("//+").unwrap();
    let mut path = re.replace_all(path, "/").to_string();
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    path
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
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
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("failed to get timestamp").as_secs();
        timestamp / unit
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Setting {
    pub rate_limit: RateLimit,
}

#[derive(Debug, Eq, PartialEq)]
pub enum CIDR {
    V4([u8; 4], u8),
    V6([u16; 8], u8),
}

#[derive(Debug, Error)]
pub enum ParseCIDRError {
    #[error("invalid format, expected ip/prefix. Got: {0}")]
    InvalidFormat(String),
    #[error("invalid ip address")]
    AddrParseError(#[from] std::net::AddrParseError),
    #[error("invalid prefix, must be a number between 0 and 32 for IPv4, 0 and 128 for IPv6. Got: {0}")]
    InvalidPrefix(String),
}

impl FromStr for CIDR {
    type Err = ParseCIDRError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Err(ParseCIDRError::InvalidFormat(s.to_string()));
        }
        let ip = parts[0].parse()?;
        let prefix = parts[1].parse::<u8>()
            .map_err(|e| ParseCIDRError::InvalidPrefix(e.to_string()))?;

        match ip {
            IpAddr::V4(ip) => {
                if prefix > 32 {
                    Err(ParseCIDRError::InvalidPrefix(prefix.to_string()))
                } else {
                    Ok(CIDR::V4(ip.octets(), prefix))
                }
            },
            IpAddr::V6(ip) => {
                if prefix > 128 {
                    Err(ParseCIDRError::InvalidPrefix(prefix.to_string()))
                } else {
                    Ok(CIDR::V6(ip.segments(), prefix))
                }
            },
        }
    }
}

impl Serialize for CIDR {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            CIDR::V4(ip, prefix) => {
                serializer.serialize_str(&format!("{}.{}.{}.{}/{}", ip[0], ip[1], ip[2], ip[3], prefix))
            },
            CIDR::V6(ip, prefix) => {
                serializer.serialize_str(&format!("{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}/{}", ip[0], ip[1], ip[2], ip[3], ip[4], ip[5], ip[6], ip[7], prefix))
            },
        }
    }
}

impl<'de> Deserialize<'de> for CIDR {
    fn deserialize<D>(deserializer: D) -> Result<CIDR, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl CIDR {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self, ip) {
            (CIDR::V4(cidr, prefix), IpAddr::V4(ip)) => {
                let mask = u32::MAX << (32 - prefix);
                let cidr = u32::from_be_bytes(*cidr);
                let ip = u32::from_be_bytes(ip.octets());
                (cidr & mask) == (ip & mask)
            },
            (CIDR::V6(cidr, prefix), IpAddr::V6(ip)) => {
                let mask = u128::MAX << (128 - prefix);
                let cidr = u128::from_be_bytes(Self::u16s_to_u8s(*cidr));
                let ip = u128::from_be_bytes(Self::u16s_to_u8s(ip.segments()));
                (cidr & mask) == (ip & mask)
            },
            _ => false,
        }
    }

    fn u16s_to_u8s(input: [u16; 8]) -> [u8; 16] {
        let mut output = [0u8; 16];
        for (i, &item) in input.iter().enumerate() {
            output[i * 2] = (item & 0xFF) as u8;         // Lower byte
            output[i * 2 + 1] = (item >> 8) as u8;       // Upper byte
        }
        output
    }
}

pub struct Router<T>(Trie<RadixTree<T>>);

pub struct Found<'a, T>(Matches<'a, T>);

impl <'a, T> Found<'a, T> {
    pub fn pattern(&self) -> &str {
        &self.0.data.pattern
    }
}

impl Deref for Found<'_, Setting> {
    type Target = Setting;

    fn deref(&self) -> &Self::Target {
        &self.0.data.data
    }
}

impl <T> Router<T> {
    pub fn matches(&self, domain: &str, path: &str) -> Option<Found<T>> {
        let route = self.0.matches(domain)?;
        route.matches(path).map(|matches| Found(matches))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_config() {
        let config_str = r#"
whitelist:
  - "46.3.240.0/24"
  - "2001:db8::/32"
difficulty: 1000000
virtual_hosts:
  - host: "example.com"
    routes:
      - path: "/"
        rate_limit:
          unit: minute
          requests_per_unit: 100
      - path: "/api"
        rate_limit:
          unit: minute
          requests_per_unit: 50
        children:
          - path: "/users"
            rate_limit:
                unit: minute
                requests_per_unit: 100
          - path: "/posts/*"
            rate_limit:
                unit: minute
                requests_per_unit: 100
  - host: "another-example.com"
    routes:
      - path: "/"
        rate_limit:
          unit: minute
          requests_per_unit: 200
      - path: "/about"
        rate_limit:
          unit: minute
          requests_per_unit: 100
        "#;

        let config: Config<Setting> = serde_yaml::from_str(config_str).expect("failed to parse config");
        println!("{:?}", config.whitelist);
        let route: Router<Setting> = config.try_into().expect("failed to convert config");

        let found = route.matches("example.com", "/api/posts/114514").expect("route not found");
        println!("{:?}", found.rate_limit);
    }

    #[test]
    fn cidr_contains() {
        let cidr: CIDR = "192.168.0.0/24".parse().unwrap();
        assert!(cidr.contains("192.168.0.250".parse().unwrap()));
        assert!(!cidr.contains("192.168.10.250".parse().unwrap()));

        let cidr: CIDR = "2001:db8::/32".parse().unwrap();
        assert!(cidr.contains("2001:db8::1".parse().unwrap()));
        assert!(cidr.contains("2001:db8::ffff".parse().unwrap()));
    }
}