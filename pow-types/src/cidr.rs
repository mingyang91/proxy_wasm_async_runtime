use std::{fmt::Display, net::IpAddr, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Eq, PartialEq)]
pub enum CIDR {
    V4([u8; 4], u8),
    V6([u16; 8], u8),
}

impl Display for CIDR {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CIDR::V4(ip, mask) => write!(f, "{}.{}.{}.{}/{}", ip[0], ip[1], ip[2], ip[3], mask),
            CIDR::V6(ip, mask) => {
                print_compressed_ip(ip, f)?;
                write!(f, "/{}", mask)
            }
        }
    }
}

fn print_compressed_ip(ip: &[u16; 8], f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let mut best_range: Option<(usize, usize)> = None;
    let mut current_range: Option<(usize, usize)> = None;

    // Find the best place to compress the zero blocks
    for (i, &segment) in ip.iter().enumerate() {
        if segment == 0 {
            if let Some((start, _)) = current_range {
                current_range = Some((start, i));
            } else {
                current_range = Some((i, i));
            }
        } else {
            if let Some((start, end)) = current_range {
                if best_range
                    .map(|(fst, snd)| (end - start) > (snd - fst))
                    .unwrap_or(true)
                {
                    best_range = current_range;
                }
            }
            current_range = None;
        }
    }

    // Final check for the best range of zeros
    if let Some((start, end)) = current_range {
        if best_range
            .map(|(fst, snd)| (end - start) > (snd - fst))
            .unwrap_or(true)
        {
            best_range = current_range;
        }
    }

    // Special case for when the entire address is "::"
    if best_range == Some((0, 7)) {
        return write!(f, "::");
    }

    let (start, end) = best_range.unwrap_or((0, 0));

    if ip[0] != 0 && best_range.is_some() && start != 0 {
        write!(f, "{:x}", ip[0])?;
    }
    // Print the IPv6 address, applying compression where needed
    for (i, &segment) in ip.iter().enumerate().skip(1) {
        if i == start || i == end {
            write!(f, ":")?;
        } else if i > start && i < end {
            continue;
        } else if i == end + 1 {
            write!(f, "{:x}", segment)?;
        } else {
            write!(f, ":{:x}", segment)?;
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum ParseCIDRError {
    #[error("invalid format, expected ip/prefix. Got: {0}")]
    InvalidFormat(String),
    #[error("invalid ip address")]
    AddrParseError(#[from] std::net::AddrParseError),
    #[error(
        "invalid prefix, must be a number between 0 and 32 for IPv4, 0 and 128 for IPv6. Got: {0}"
    )]
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
        let prefix = parts[1]
            .parse::<u8>()
            .map_err(|e| ParseCIDRError::InvalidPrefix(e.to_string()))?;

        match ip {
            IpAddr::V4(ip) => {
                if prefix > 32 {
                    Err(ParseCIDRError::InvalidPrefix(prefix.to_string()))
                } else {
                    Ok(CIDR::V4(ip.octets(), prefix))
                }
            }
            IpAddr::V6(ip) => {
                if prefix > 128 {
                    Err(ParseCIDRError::InvalidPrefix(prefix.to_string()))
                } else {
                    Ok(CIDR::V6(ip.segments(), prefix))
                }
            }
        }
    }
}

impl Serialize for CIDR {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
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
            }
            (CIDR::V6(cidr, prefix), IpAddr::V6(ip)) => {
                let mask = u128::MAX << (128 - prefix);
                let cidr = u128::from_be_bytes(Self::u16s_to_u8s(*cidr));
                let ip = u128::from_be_bytes(Self::u16s_to_u8s(ip.segments()));
                (cidr & mask) == (ip & mask)
            }
            _ => false,
        }
    }

    fn u16s_to_u8s(input: [u16; 8]) -> [u8; 16] {
        let mut output = [0u8; 16];
        for (i, &item) in input.iter().enumerate() {
            output[i * 2] = (item & 0xFF) as u8; // Lower byte
            output[i * 2 + 1] = (item >> 8) as u8; // Upper byte
        }
        output
    }
}
