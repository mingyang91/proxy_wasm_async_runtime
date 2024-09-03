pub mod runtime;
pub mod chain;

use chain::bytearray32::ByteArray32;
use log::info;
use proxy_wasm::traits::*;
use proxy_wasm::types::*;
use runtime::route::config::Config;
use runtime::route::config::Router;
use runtime::route::config::Setting;
use runtime::Ctx;
use runtime::HookHolder;
use runtime::HttpHook;
use runtime::Response;
use runtime::{Runtime, RuntimeBox};
use sha2::Digest;
use std::rc::Rc;
use std::sync::OnceLock;
use chain::btc::BTC;

static BTC: OnceLock<BTC> = OnceLock::new();

fn get_btc() -> &'static BTC {
    BTC.get_or_init(BTC::new)
}

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);

    proxy_wasm::set_root_context(move |_| -> Box<dyn RootContext> { 
        Box::new(RuntimeBox::new(Plugin { router: Rc::new(None) }))
    });
    proxy_wasm::set_http_context(|context_id, _| -> Box<dyn HttpContext> { 
        Box::new(HookHolder::<Hook>::new(context_id))
     });
}}

#[derive(Clone)]
struct Plugin {
    router: Rc<Option<Router<Setting>>>
}

impl Context for Plugin {}
impl Runtime for Plugin {
    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        info!("Hello from WASM");
        let this = self.clone();
        runtime::spawn_local(async move {
            get_btc().start(&this).await;
        });
        true
    }

    fn on_configure(&mut self, configuration: Option<Vec<u8>>) -> bool {
        let Some(config_bytes) = configuration else {
            return false
        };

        let config: Config<Setting> = match serde_yaml::from_slice(&config_bytes) {
            Ok(config) => config,
            Err(e) => {
                log::error!("failed to parse configuration: {}\n raw config: {}", e, String::from_utf8(config_bytes).expect("failed to read raw config into utf8 string"));
                return false;
            }
        };

        let router: Router<Setting> = match config.try_into() {
            Ok(router) => router,
            Err(e) => {
                log::error!("failed to convert configuration: {}\n raw config: {}", e, String::from_utf8(config_bytes).expect("failed to read raw config into utf8 string"));
                return false;
            }
        };

        self.router = Rc::new(Some(router));
        true
    }
}


pub struct Hook { 
    ctx: Ctx,
}

impl From<u32> for Hook {
    fn from(id: u32) -> Self {
        Self {
            ctx: Ctx::new(id),
        }
    }
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

fn too_many_request() -> Error {
    let Some(last_hash) = get_btc().get_latest_hash() else {
        return Error::status("failed to get latest hash", Status::NotFound)
    };
    let current = last_hash.as_str().try_into().expect("failed to parse latest hash");
    let difficulty = get_difficulty(1_000_000);
    let body = DifficultyResponse {
        current,
        difficulty
    };
    Error::response(Response {
        code: 200,
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

impl HttpHook for Hook {
    async fn on_request_headers(&self, _num_headers: usize, _end_of_stream: bool) -> Result<(), impl Into<Response>> {
        let Some(path) = self.ctx.get_http_request_header(":path")
            .map_err(|s| Error::status("failed to get path", s))? else {
            return Err(forbidden("failed to get path from request".to_string()));
        };

        let Some(addr) = self.ctx.get_client_address()
            .map_err(|s| Error::status("failed to get client address", s))? else {
            return Err(forbidden("failed to get client address from request".to_string()));
        };

        log::info!("request from: {}", addr);

        return match path.as_str() {
            "/api/difficulty" => Err(too_many_request()),
            _ => {
                let nonce = self.ctx.get_http_request_header("X-Nonce")
                    .map_err(|s| Error::status("failed to get nonce", s))?
                    .ok_or_else(too_many_request)?;
                let nonce = hex::decode(nonce)
                    .map_err(|s| Error::response(
                        Response {
                            code: 400,
                            headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                            body: Some(format!("invalid nonce: {}", s).into_bytes()),
                            trailers: vec![],
                        }
                    ))?;

                let data = self.ctx.get_http_request_header("X-Data")
                    .map_err(|s| Error::status("failed to get data", s))?
                    .ok_or_else(too_many_request)?;
                
                let difficulty = get_difficulty(1_000);


                if valid_nonce(data.as_bytes(), difficulty, &nonce) {
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
}