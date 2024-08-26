pub mod runtime;
pub mod chain;

use chain::bytearray32::ByteArray32;
use log::info;
use proxy_wasm::traits::*;
use proxy_wasm::types::*;
use runtime::Ctx;
use runtime::HookHolder;
use runtime::HttpHook;
use runtime::Response;
use runtime::{Runtime, RuntimeBox};
use sha2::Digest;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::time::Duration;
use chain::btc::BTC;

static BTC: OnceLock<RwLock<BTC>> = OnceLock::new();

fn get_btc() -> &'static RwLock<BTC> {
    BTC.get_or_init(|| RwLock::new(BTC::new()))
}

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);

    proxy_wasm::set_root_context(move |_| -> Box<dyn RootContext> { 
        Box::new(RuntimeBox::new(Plugin {}))
    });
    proxy_wasm::set_http_context(|context_id, _| -> Box<dyn HttpContext> { 
        Box::new(HookHolder::<Hook>::new(context_id))
     });
}}

#[derive(Default)]
struct HttpAuthRandom { token: Option<u32> }

impl HttpContext for HttpAuthRandom {
    fn on_http_request_headers(&mut self, _: usize, _: bool) -> Action {
        let token = self.dispatch_http_call(
            "httpbin",
            vec![
                (":method", "GET"),
                (":path", "/bytes/1"),
                (":authority", "httpbin.org"),
            ],
            None,
            vec![],
            Duration::from_secs(1),
        )
        .unwrap();
        self.token.replace(token);
        Action::Pause
    }

    fn on_http_response_headers(&mut self, _: usize, _: bool) -> Action {
        self.set_http_response_header("Powered-By", Some("proxy-wasm"));
        Action::Continue
    }
}

impl Context for HttpAuthRandom {
    fn on_http_call_response(&mut self, token: u32, _: usize, body_size: usize, _: usize) {
        if Some(token) != self.token {
            return;
        }
        if let Some(body) = self.get_http_call_response_body(0, body_size) {
            if !body.is_empty() && body[0] % 2 == 0 {
                info!("Access granted.");
                self.resume_http_request();
                return;
            }
        }
        info!("Access forbidden.");
        self.send_http_response(
            403,
            vec![("Powered-By", "proxy-wasm")],
            Some(b"Access forbidden.\n"),
        );
    }
}


#[derive(Clone)]
struct Plugin {}

impl Context for Plugin {}
impl Runtime for Plugin {
    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        info!("Hello from WASM");
        let this = self.clone();
        runtime::spawn_local(async move {
            get_btc()
                .read()
                .expect("failed to read BTC")
                .start(&this).await;
        });
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
    Other { reason: String, error: Box<dyn std::error::Error> },
}

impl Error {
    fn status(reason: impl Into<String>, status: proxy_wasm::types::Status) -> Self {
        Error::Status { reason: reason.into(), status }
    }

    fn response(response: Response) -> Self {
        Error::Response(response)
    }

    fn other(reason: impl Into<String>, error: impl Into<Box<dyn std::error::Error>>) -> Self {
        Error::Other { reason: reason.into(), error: error.into() }
    }
}

impl Into<Response> for Error {
    fn into(self) -> Response {
        match self {
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

fn miss_nonce() -> Response {
    Response {
        code: 400,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: Some("missing nonce".to_string().into_bytes()),
        trailers: vec![],
    }
}

impl HttpHook for Hook {
    async fn on_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Result<(), impl Into<Response>> {
        let Some(path) = self.ctx.get_http_request_header(":path").unwrap() else {
            return Ok(())
        };

        return match path.as_str() {
            "/api/difficulty" => {
                let last_hash = {
                    get_btc().read()
                        .map_err(|s| Error::other("failed to get lock", s))?
                        .get_latest_hash()
                        .ok_or(Error::status("failed to get latest hash", Status::NotFound))?
                };
                let current = last_hash.as_str().try_into().expect("failed to parse latest hash");
                let difficulty = get_difficulty(1_000_000);
                let body = DifficultyResponse { 
                    current,
                    difficulty 
                };
                Err(Error::response(Response {
                    code: 200,
                    headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                    body: Some(serde_json::to_string(&body).expect("failed to serialize difficulty").into_bytes()),
                    trailers: vec![],
                }))
            },
            _ => {
                let nonce = self.ctx.get_http_request_header("X-Nonce")
                    .map_err(|s| Error::status("failed to get nonce", s))?
                    .ok_or(Error::response(miss_nonce()))?;
                let data = self.ctx.get_http_request_header("X-Data")
                    .map_err(|s| Error::status("failed to get data", s))?
                    .ok_or(Error::response(Response {
                        code: 400,
                        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                        body: Some("missing data".to_string().into_bytes()),
                        trailers: vec![],
                    }))?;
                
                let difficulty = get_difficulty(1_000);
                let nonce = hex::decode(nonce)
                    .map_err(|s| Error::response(
                        Response {
                            code: 400,
                            headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                            body: Some(format!("invalid nonce: {}", s).into_bytes()),
                            trailers: vec![],
                        }
                    ))?;
                    
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
