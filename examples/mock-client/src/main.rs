use reqwest::Client;
use sha2::Digest;
use pow_types::bytearray32::ByteArray32;

#[tokio::main]
async fn main() {
    let mut tasks = vec![];
    for _ in 0..12 {
        tasks.push(tokio::spawn(async move {
            loop {
                let start = std::time::Instant::now();
                if let Err(e) = single_request().await {
                    println!("Error: {}", e);
                } else {
                    println!("time: {}sec", start.elapsed().as_secs());
                }
            }
        }));
    }

    futures::future::join_all(tasks).await;
}

#[derive(Debug, serde::Deserialize)]
struct PoW {
    current: ByteArray32,
    difficulty: ByteArray32,
    #[allow(dead_code)]
    message: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Error(String);


async fn single_request() -> Result<(), Box<dyn std::error::Error>> {
    let address = "bc1p5d7rjq7g6rdk2yhzks9smlaqtedr4dekq08ge8ztwac72sfr9rusxg3297";
    let path = format!("/ip?address={}", address);
    let url = format!("http://localhost:10000{}", path);

    let response = Client::new()
        .get(&url)
        .header("Host", "httpbin.org")
        .send()
        .await?;

    let mut pow: PoW = match response.status().as_u16() {
        429 => response.json().await?,
        403 => { return Err(Box::new(Error(response.text().await?))) },
        _ => { 
            let body = response.text().await?;
            println!("Success: {}", body);
            return Ok(())
        },
    };
    
    loop {
        println!("difficulty: {:?}", pow.difficulty);

        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("failed to get timestamp").as_secs();
        let mut data = pow.current.as_bytes().to_vec();
        data.extend(timestamp.to_be_bytes());
        data.extend(path.as_bytes());


        let nonce = tokio::task::spawn_blocking(move || {
            mine(&data, pow.difficulty)
        }).await.expect("join failed");

        let response = Client::new()
            .get(&url)
            .header("Host", "httpbin.org")
            .header("X-PoW-Timestamp", timestamp.to_string())
            .header("X-PoW-Nonce", print_hex(&nonce))
            .header("X-PoW-Base", print_hex(pow.current.as_bytes()))
            .send()
            .await?;

        if response.status() != 429 || response.status() != 403 {
            let body = response.text().await?;
            println!("Success: {}", body);
            return Ok(())
        }
        pow = match response.status().as_u16() {
            429 => response.json().await?,
            403 => { return Err(Box::new(Error(response.text().await?))) },
            _ => { 
                let body = response.text().await?;
                println!("Success: {}", body);
                return Ok(())
            },
        };
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

