mod utils;

use sha2::Digest;
use pow_types::bytearray32::ByteArray32;
use wasm_bindgen::prelude::*;
use serde_wasm_bindgen::{from_value, to_value};


fn init_log() {
    #[cfg(feature = "console_log")]
    console_log::init().expect("error initializing log");
}

#[wasm_bindgen]
pub fn startup() {
    init_log();
}

#[derive(Debug, serde::Deserialize)]
struct MineArgs {
    path: String,
    current: ByteArray32,
    difficulty: ByteArray32,
    timestamp: u64,
}

#[derive(Debug, serde::Serialize)]
struct MineResult {
    #[serde(rename = "X-PoW-Nonce")]
    nonce: String,
    #[serde(rename = "X-PoW-Timestamp")]
    timestamp: String,
    #[serde(rename = "X-PoW-Base")]
    base: String,
}

#[wasm_bindgen]
pub fn mine(args: JsValue) -> Result<JsValue, JsError> {
    let args = match from_value(args) {
        Ok(args) => args,
        Err(err) => return Err(JsError::new(&format!("{}", err))),
    };

    let result = mine_impl(args);
    
    match to_value(&result) {
        Ok(value) => Ok(value),
        Err(err) => Err(JsError::new(&format!("{}", err))),
    }
}

fn mine_impl(args: MineArgs) -> MineResult {
    let mut data = args.current.as_bytes().to_vec();
    data.extend(args.timestamp.to_be_bytes());
    data.extend(args.path.as_bytes());
    loop {
        let nonce = rand::random::<[u8; 8]>();
        if valid_nonce(&data, args.difficulty, &nonce) {
            let hex_nonce = format!("{:x}", LowerHexSlice(&nonce));
            log::debug!("found nonce: {}", hex_nonce);
            return MineResult {
                nonce: hex_nonce,
                timestamp: args.timestamp.to_string(),
                base: format!("{:x}", LowerHexSlice(args.current.as_bytes())),
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

struct LowerHexSlice<'a, T>(&'a [T]);

impl<T> std::fmt::LowerHex for LowerHexSlice<'_, T>
where
    T: std::fmt::LowerHex, {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}
