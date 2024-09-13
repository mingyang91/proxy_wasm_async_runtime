pub mod auth_identity;
pub mod config;

use std::{net::SocketAddr, sync::Arc};

use auth_identity::{AuthFactors, AuthIdentity};
use config::{Config, Setting};
use pow_runtime::{response::Response, Ctx, HttpHook, Runtime, RuntimeBox};
use pow_types::{cidr::CIDR, config::Router};
use proxy_wasm::{
    traits::{Context, RootContext},
    types::LogLevel,
};
use secp256k1::{ecdsa::Signature, PublicKey};

const HEADER_PUBLIC_KEY_NAME: &str = "X-Auth-Public-Key";
const HEADER_SIGNATURE_NAME: &str = "X-Auth-Signature";
const HEADER_TIMESTAMP_NAME: &str = "X-Auth-Timestamp";

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);
    proxy_wasm::set_root_context(move |context_id| -> Box<dyn RootContext> {
        Box::new(RuntimeBox::new(Plugin { _context_id: context_id, inner: None }))
    });
}}

#[derive(Debug)]
#[allow(dead_code)]
enum Error {
    Status {
        reason: String,
        status: proxy_wasm::types::Status,
    },
    Response(Response),
    Other {
        reason: String,
        error: Box<dyn std::error::Error>,
    },
}

#[allow(dead_code)]
impl Error {
    fn status(reason: &str, status: proxy_wasm::types::Status) -> Self {
        Self::Status {
            reason: reason.to_owned(),
            status,
        }
    }

    fn response(response: Response) -> Self {
        Self::Response(response)
    }

    fn other(reason: &str, error: Box<dyn std::error::Error>) -> Self {
        Self::Other {
            reason: reason.to_owned(),
            error,
        }
    }
}

impl From<Error> for Response {
    fn from(val: Error) -> Self {
        match val {
            Error::Response(response) => {
                log::debug!("reject request with response, {:?}", response.code);
                response
            }
            Error::Status { reason, status } => {
                let msg = format!("{:?}: {}", status, reason);
                log::warn!("failed hostcall with error, {}", msg);
                Response {
                    code: 500,
                    headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                    body: Some(msg.into_bytes()),
                    trailers: vec![],
                }
            }
            Error::Other { reason, error } => {
                let msg = format!("{}: {}", error, reason);
                log::warn!("failed unknow error, {}", msg);
                Response {
                    code: 500,
                    headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                    body: Some(msg.into_bytes()),
                    trailers: vec![],
                }
            }
        }
    }
}

struct Inner {
    router: Router<Setting>,
    whitelist: Vec<CIDR>,
}

#[derive(Clone)]
struct Plugin {
    _context_id: u32,
    inner: Option<Arc<Inner>>,
}

impl Context for Plugin {}
impl Runtime for Plugin {
    type Hook = Hook;

    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        log::info!("Auth filter starting...");
        true
    }

    fn on_configure(&mut self, _configuration: Option<Vec<u8>>) -> bool {
        log::info!("Auth filter configuring...");
        let Some(config_bytes) = _configuration else {
            log::error!("missing configuration");
            return false;
        };

        let mut config: Config<Setting> = match serde_json::from_slice(&config_bytes) {
            Ok(config) => config,
            Err(e) => {
                log::error!(
                    "failed to parse configuration, {}\nraw config: {}",
                    e,
                    String::from_utf8(config_bytes)
                        .expect("failed to convert raw config to string")
                );
                return false;
            }
        };

        proxy_wasm::set_log_level(config.log_level.map(Into::into).unwrap_or(LogLevel::Trace));

        let whitelist = config.whitelist.take().unwrap_or_default();

        let router: Router<Setting> = match config.virtual_hosts.try_into() {
            Ok(router) => router,
            Err(e) => {
                log::error!(
                    "failed to convert configuration: {}\n raw config: {}",
                    e,
                    String::from_utf8(config_bytes)
                        .expect("failed to read raw config into utf8 string")
                );
                return false;
            }
        };

        self.inner = Some(Arc::new(Inner { router, whitelist }));
        log::info!("Auth filter configured...");
        true
    }

    fn create_http_context(&self, _context_id: u32) -> Option<Self::Hook> {
        Some(Hook {
            ctx: Ctx::new(_context_id),
            plugin: self.inner.clone().expect("plugin not configured"),
        })
    }
}

#[derive(Debug, serde::Serialize)]
pub struct UnauthorizedResponse {
    error: String,
    message: String,
}

fn unauthorized(error: &str) -> Error {
    let body = UnauthorizedResponse {
        error: error.to_owned(),
        message: "Lacks valid authentication credentials for the requested resource".to_string(),
    };
    Error::response(Response {
        code: 429,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: Some(
            serde_json::to_string(&body)
                .expect("failed to serialize response")
                .into_bytes(),
        ),
        trailers: vec![],
    })
}

fn forbidden(message: &str) -> Error {
    let body = serde_json::json!({ "message": message });
    Error::response(Response {
        code: 403,
        headers: vec![("Content-Type".to_string(), "text/json".to_string())],
        body: Some(body.to_string().into_bytes()),
        trailers: vec![],
    })
}

pub struct Hook {
    ctx: Ctx,
    plugin: Arc<Inner>,
}

impl Hook {
    fn get_client_addr(&self) -> Result<String, Error> {
        self.ctx
            .get_client_address()
            .map_err(|s| Error::status("failed to get client address", s))?
            .ok_or_else(|| forbidden("failed to get client address from request"))
    }

    fn get_header(&self, key: &str) -> Result<String, Error> {
        self.ctx
            .get_http_request_header(key)
            .map_err(|s| Error::status(&format!("failed to get header: {}", key), s))?
            .ok_or_else(|| forbidden(&format!("missing header: {}", key)))
    }

    fn get_path(&self) -> Result<String, Error> {
        self.ctx
            .get_http_request_path()
            .map_err(|s| Error::status("failed to get path", s))
    }
}

impl HttpHook for Hook {
    fn filter_name() -> Option<&'static str> {
        Some("auth")
    }

    async fn on_request_headers(
        &self,
        _num_headers: usize,
        _end_of_stream: bool,
    ) -> Result<(), impl Into<Response>> {
        let addr = self.get_client_addr()?;
        let addr: SocketAddr = addr
            .parse()
            .map_err(|s| forbidden(&format!("invalid client address {}: {}", s, addr)))?;
        if self
            .plugin
            .whitelist
            .iter()
            .any(|cidr| cidr.contains(addr.ip()))
        {
            return Ok(());
        }

        let host = self.get_header(":authority")?;
        let path = self.get_path()?;

        log::debug!("{} -> {}{}", addr, host, path);

        let Some(found) = self.plugin.router.matches(&host, &path) else {
            log::debug!("no matched route found, skip auth check");
            return Ok(());
        };

        let public_key: PublicKey = self
            .get_header(HEADER_PUBLIC_KEY_NAME)
            .map_err(|_| unauthorized(&format!("Missing {} in header", HEADER_PUBLIC_KEY_NAME)))?
            .parse()
            .map_err(|e| unauthorized(&format!("Invalid public key: {}", e)))?;

        match found.grants.get(&public_key) {
            Some(trusted_name) => {
                log::debug!("found public key in grants: {}, continue...", trusted_name);
            }
            None => return Err(unauthorized("Public key not found in grants")),
        }

        let signature: Signature = self
            .get_header(HEADER_SIGNATURE_NAME)
            .map_err(|_| unauthorized(&format!("Missing {} in header", HEADER_SIGNATURE_NAME)))?
            .parse()
            .map_err(|e| {
                unauthorized(&format!(
                    "Invalid signature, expect a DER format string: {}",
                    e
                ))
            })?;

        let timestamp = self
            .get_header(HEADER_TIMESTAMP_NAME)
            .map_err(|_| unauthorized(&format!("Missing {} in header", HEADER_TIMESTAMP_NAME)))?;

        let timestamp = timestamp
            .parse::<u64>()
            .map_err(|_| unauthorized("Invalid timestamp"))?;

        let factors = AuthFactors::new(&path, timestamp);
        let auth_identity = AuthIdentity::new(&public_key, factors, &signature);
        auth_identity
            .verify()
            .map_err(|e| unauthorized(&format!("Failed to verify signature: {}", e)))
    }
}

#[cfg(test)]
mod test {
    use hex_literal::hex;
    use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
    use sha2::{Digest, Sha256};

    fn digest<D>(data: D) -> [u8; 32]
    where
        D: AsRef<[u8]>,
    {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    #[test]
    fn test() {
        let hex_secret = hex!("3f880ce0892ac66019804c80292d4e90a38aa70a9dabad3f4314bf050f492afc");
        let secret = SecretKey::from_slice(&hex_secret).unwrap();
        println!("{:?}", secret);
        let secp = Secp256k1::new();
        let pub_key = PublicKey::from_secret_key(&secp, &secret);
        println!("{:?}", pub_key);
        println!("{:?}", pub_key.serialize());
        println!("{:?}", PublicKey::from_slice(&pub_key.serialize()));

        let msg_plain = b"hello world";
        let digest = digest(msg_plain);
        let msg = Message::from_digest(digest);

        let sign = secp.sign_ecdsa(&msg, &secret);
        println!("{:?}", sign);

        let verify = secp.verify_ecdsa(&msg, &sign, &pub_key);
        println!("{:?}", verify);
        assert!(verify.is_ok());
    }
}
