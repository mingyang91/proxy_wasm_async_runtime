use proxy_wasm::types;
use serde::{Deserialize, Serialize};

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl From<LogLevel> for types::LogLevel {
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
