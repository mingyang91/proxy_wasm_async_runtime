use reqwest::Client;
use sha2::Digest;
use std::fmt::{Formatter, LowerHex};

pub type ByteArray32 = FixedByteArray<32>;

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub struct FixedByteArray<const N: usize>([u8; N]);

impl <const N: usize> FixedByteArray<N> {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl <const N: usize> From<&[u8; N]> for FixedByteArray<N> {
    fn from(bytes: &[u8; N]) -> Self {
        let mut result = [0; N];
        result.copy_from_slice(bytes);
        FixedByteArray(result)
    }
}

impl <const N: usize> TryFrom<&str> for FixedByteArray<N> {
    type Error = &'static str;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        if s.len() != N * 2 {
            return Err("invalid length");
        }
        let mut bytes = [0; N];
        for (i, item) in bytes.iter_mut().enumerate() {
            let start = i * 2;
            let end = start + 2;
            *item = u8::from_str_radix(&s[start..end], 16)
                .map_err(|_| "invalid hex")?;
        }
        Ok(FixedByteArray(bytes))
    }
}

impl <const N: usize> serde::Serialize for FixedByteArray<N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{:x}", self))
    }
}

impl <'de, const N: usize> serde::Deserialize<'de> for FixedByteArray<N> {
    fn deserialize<D>(deserializer: D) -> Result<FixedByteArray<N>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FixedByteArray::<N>::try_from(s.as_str()).map_err(serde::de::Error::custom)
    }
}

impl <const N: usize> LowerHex for FixedByteArray<N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut tasks = vec![];
    for _ in 0..12 {
        tasks.push(tokio::spawn(async move {
            loop {
                let start = std::time::Instant::now();
                single_request().await;
                println!("time: {}sec", start.elapsed().as_secs());
            }
        }));
    }

    futures::future::join_all(tasks).await;
}

#[derive(Debug, serde::Deserialize)]
struct PoW {
    current: ByteArray32,
    difficulty: ByteArray32,
}


async fn single_request() -> Result<(), Box<dyn std::error::Error>> {
    let address = "bc1p5d7rjq7g6rdk2yhzks9smlaqtedr4dekq08ge8ztwac72sfr9rusxg3297";

    let response = Client::new()
        .get("http://localhost:10000/ip")
        .header("Host", "httpbin.org")
        .send()
        .await?;

    if response.status() != 429 {
        let body = response.text().await?;
        println!("Success: {}", body);
        return Ok(())
    }
    let mut pow: PoW = response.json().await?;
    loop {
        println!("difficulty: {:?}", pow.difficulty);

        let mut data = pow.current.as_bytes().to_vec();
        data.extend(address.as_bytes());


        let nonce = tokio::task::spawn_blocking(move || {
            mine(&data, pow.difficulty)
        }).await.expect("join failed");

        let response = Client::new()
            .get("http://localhost:10000/ip")
            .header("Host", "httpbin.org")
            .header("X-Nonce", print_hex(&nonce))
            .header("X-Data", address)
            .header("X-Last", print_hex(pow.current.as_bytes()))
            .send()
            .await?;

        if response.status() != 429 {
            let body = response.text().await?;
            println!("Success: {}", body);
            return Ok(())
        }

        pow = response.json().await?;
    }
}

fn mine(data: &[u8], difficulty: ByteArray32) -> [u8; 8] {
    loop {
        let nonce = rand::random::<[u8; 8]>();
        if valid_nonce(data, difficulty, &nonce) {
            println!("found nonce: {}", print_hex(&nonce));
            return nonce
        }
    }
}

fn print_hex(bytes: &[u8]) -> String {
    format!("{:x}", LowerHexSlice(bytes))
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

