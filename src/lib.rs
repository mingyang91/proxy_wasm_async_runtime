pub mod runtime;
pub mod chain;

use chain::bytearray32::ByteArray32;
use log::info;
use proxy_wasm::traits::*;
use proxy_wasm::types::*;
use runtime::counter_bucket::CounterBucket;
use runtime::route::config::Config;
use runtime::route::config::Router;
use runtime::route::config::Setting;
use runtime::route::config::CIDR;
use runtime::Ctx;
use runtime::HttpHook;
use runtime::response::Response;
use runtime::{Runtime, RuntimeBox};
use sha2::Digest;
use std::net::SocketAddr;
use std::sync::Arc;
use chain::btc::BTC;

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);

    proxy_wasm::set_root_context(move |context_id| -> Box<dyn RootContext> { 
        Box::new(RuntimeBox::new(Plugin { context_id, inner: None }))
    });
}}


struct Inner {
    btc: BTC,
    router: Router<Setting>,
    counter_bucket: CounterBucket,
    whitelist: Vec<CIDR>,
    difficulty: u64,
}

#[derive(Clone)]
struct Plugin {
    context_id: u32,
    inner: Option<Arc<Inner>>,
}

impl Context for Plugin {}
impl Runtime for Plugin {
    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        info!("PoW filter starting...");
        true
    }

    fn on_configure(&mut self, configuration: Option<Vec<u8>>) -> bool {
        info!("PoW filter configuring...");
        let Some(config_bytes) = configuration else {
            return false
        };

        let mut config: Config<Setting> = match serde_yaml::from_slice(&config_bytes) {
            Ok(config) => config,
            Err(e) => {
                log::error!("failed to parse configuration: {}\n raw config: {}", e, String::from_utf8(config_bytes).expect("failed to read raw config into utf8 string"));
                return false;
            }
        };

        let whitelist = config.whitelist.take().unwrap_or_default();
        let difficulty = config.difficulty;

        let router: Router<Setting> = match config.try_into() {
            Ok(router) => router,
            Err(e) => {
                log::error!("failed to convert configuration: {}\n raw config: {}", e, String::from_utf8(config_bytes).expect("failed to read raw config into utf8 string"));
                return false;
            }
        };

        self.inner = Some(Arc::new(Inner {
            btc: BTC::new(),
            router,
            counter_bucket: CounterBucket::new(self.context_id, "rate_limit"),
            whitelist,
            difficulty,
        }));
        info!("PoW filter configured");
        true
    }
    
    type Hook = Hook;
    
    fn create_http_context(&self, _context_id: u32) -> Option<Self::Hook> {
        Some(Hook { 
            ctx: Ctx::new(_context_id),
            plugin: self.inner.clone().expect("plugin not initialized"),
        })
    }
}


pub struct Hook { 
    ctx: Ctx,
    plugin: Arc<Inner>,
}

fn transform_u64_to_u8_array(mut value: u64) -> [u8; 8] {
    let mut result = [0; 8];
    for i in 0..8 {
        result[7 - i] = (value & 0xff) as u8;
        value >>= 8;
    }
    result
}

/// Get the difficulty target as a big-endian 256-bit number.
/// The `level` parameter represents the number of leading zero bits required.
fn get_difficulty(level: u64) -> ByteArray32 {
    let mut difficulty = [0xff; 32];
    let initial = u64::MAX / level;
    let initial_bytes = transform_u64_to_u8_array(initial);
    difficulty[0..8].clone_from_slice(&initial_bytes);
    (&difficulty).into()
}

#[derive(serde::Serialize)]
struct DifficultyResponse {
    current: ByteArray32,
    difficulty: ByteArray32,
}

#[derive(Debug)]
enum Error {
    Status { reason: String, status: proxy_wasm::types::Status },
    Response(Response),
    #[allow(dead_code)]
    Other { reason: String, error: Box<dyn std::error::Error> },
}

impl Error {
    fn status(reason: impl Into<String>, status: proxy_wasm::types::Status) -> Self {
        Error::Status { reason: reason.into(), status }
    }

    fn response(response: Response) -> Self {
        Error::Response(response)
    }

    #[allow(dead_code)]
    fn other(reason: impl Into<String>, error: impl Into<Box<dyn std::error::Error>>) -> Self {
        Error::Other { reason: reason.into(), error: error.into() }
    }
}

impl From<Error> for Response {
    fn from(val: Error) -> Self {
        match val {
            Error::Response(response) => response,
            Error::Status { reason, status } => {
                let msg = format!("{}: {:?}", reason, status);
                Response {
                    code: 500,
                    headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                    body: Some(msg.into_bytes()),
                    trailers: vec![],
                }
            },
            Error::Other { reason, error } => {
                let msg = format!("{}: {}", reason, error);
                Response {
                    code: 500,
                    headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                    body: Some(msg.into_bytes()),
                    trailers: vec![],
                }
            },
        }
    }
}

fn too_many_request(current: ByteArray32, difficulty: u64) -> Error {
    let target = get_difficulty(difficulty);
    let body = DifficultyResponse {
        current,
        difficulty: target
    };
    Error::response(Response {
        code: 429,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: Some(serde_json::to_string(&body).expect("failed to serialize difficulty").into_bytes()),
        trailers: vec![],
    })
}

fn forbidden(message: String) -> Error {
    let body = serde_json::json!({ "message": message });
    Error::response(Response {
        code: 403,
        headers: vec![("Content-Type".to_string(), "text/json".to_string())],
        body: Some(body.to_string().into_bytes()),
        trailers: vec![],
    })
}

impl Hook {
    fn get_header(&self, key: &str) -> Result<String, Error> {
        self.ctx.get_http_request_header(key)
            .map_err(|s| Error::status(format!("failed to get header: {}", key), s))?
            .ok_or_else(|| forbidden(format!("missing header: {}", key)))
    }

    fn get_client_address(&self) -> Result<String, Error> {
        self.ctx.get_client_address()
            .map_err(|s| Error::status("failed to get client address", s))?
            .ok_or_else(|| forbidden("failed to get client address from request".to_string()))
    }

    fn get_current_hash(&self) -> Result<ByteArray32, Error> {
        let Some(last_hash) = self.plugin.btc.get_latest_hash() else {
            return Err(Error::status("failed to get latest hash", Status::NotFound))
        };

        last_hash.as_str().try_into()
            .map_err(|e| Error::other("failed to parse latest hash, maybe mempool return malformed hash?", e))
    }
}

impl HttpHook for Hook {
    async fn on_request_headers(&self, _num_headers: usize, _end_of_stream: bool) -> Result<(), impl Into<Response>> {
        let addr = self.get_client_address()?;
        let addr: SocketAddr = addr.parse().map_err(|s| forbidden(format!("invalid client address {}: {}", s, addr)))?;
        if self.plugin.whitelist.iter().any(|cidr| cidr.contains(addr.ip())) {
            return Ok(());
        }
        let host = self.get_header(":authority")?;
        let path = self.get_header(":path")?;

        let Some(found) = self.plugin.router.matches(&host, &path) else {
            return Ok(())
        };

        let key = format!("{}:{}:{}{}", addr.ip(), found.rate_limit.current_bucket(), host, found.pattern());
        let counter = self.plugin.counter_bucket.get(&key).map_err(|s| Error::other("failed to get counter", s))?;
        let difficulty = counter / found.rate_limit.requests_per_unit as u64 * self.plugin.difficulty;
        let current = self.get_current_hash()?;
        log::debug!("key: {}, counter: {}, difficulty: {}", key, counter, difficulty);

        return match path.as_str() {
            "/api/difficulty" => Err(too_many_request(current, difficulty)),
            _ => {
                if difficulty == 0 {
                    self.plugin.counter_bucket.inc(&key, 1);
                    return Ok(());
                }

                let target = get_difficulty(difficulty);

                let nonce = self.get_header("X-Nonce")
                    .map_err(|_| too_many_request(current, difficulty))?;

                let nonce = hex::decode(nonce)
                    .map_err(|s| forbidden(format!("invalid nonce: {}", s)))?;

                let last = self.get_header("X-Last")
                    .map_err(|_| too_many_request(current, difficulty))?;

                if !self.plugin.btc.check_in_list(&last) {
                    return Err(too_many_request(current, difficulty))
                }

                let last: ByteArray32 = last.as_str().try_into()
                    .map_err(|e| forbidden(format!("failed to parse last hash: {}", e)))?;

                let data = self.get_header("X-Data")
                    .map_err(|_| too_many_request(current, difficulty))?;

                let mut final_data = last.as_bytes().to_vec();
                final_data.extend(data.as_bytes());
                if valid_nonce(&final_data, target, &nonce) {
                    self.plugin.counter_bucket.inc(&key, 1);
                    Ok(())
                } else {
                    Err(Error::response(Response {
                        code: 400,
                        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                        body: Some("invalid nonce".to_string().into_bytes()),
                        trailers: vec![],
                    }))
                }
            }
        }
    }
}

fn valid_nonce(data: &[u8], difficulty: ByteArray32, nonce: &[u8]) -> bool {
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    hasher.update(nonce);
    let hash = hasher.finalize();
    let slice: &[u8; 32] = &hash.into();
    let target: ByteArray32 = slice.into();
    target <= difficulty
}

#[cfg(test)]
mod test {
    use crate::{chain::bytearray32::ByteArray32, valid_nonce};

    #[test]
    fn mine() {
        let last: ByteArray32 = "000000000000000000010915948e0d6b2c40aa4144ed4277f978e231f4c44732"
            .try_into()
            .expect("failed to parse last hash");
        // 000010c6f7a0b5edffffffffffffffffffffffffffffffffffffffffffffffff
        let difficulty: ByteArray32 = "000010c6f7a0b5edffffffffffffffffffffffffffffffffffffffffffffffff"
            .try_into()
            .expect("failed to parse difficulty");

        loop {
            let nonce = rand::random::<[u8; 8]>();
            if valid_nonce(last.as_bytes(), difficulty, &nonce) {
                print!("found nonce:");
                print_hex(&nonce);
                println!();
                break;
            }
        }
    }

    fn print_hex(bytes: &[u8]) {
        for byte in bytes {
            print!("{:02x}", byte);
        }
    }

    #[test]
    fn decode() {
        let nonce = "aaed9b41fcf6dc5";
        let hex = hex::decode(nonce).expect("invalid hex");
        print_hex(&hex);
    }
}