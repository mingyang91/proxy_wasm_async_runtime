use std::{collections::VecDeque, time::Duration};

use log::{debug, info, warn};
use proxy_wasm::types::Status;

use crate::runtime::{timeout::sleep, Runtime};

pub struct BTC {
    pub recent_hash_list: VecDeque<String>,
    state: State,
}

enum State {
    Initial,
    Running,
    Stopped,
}

impl BTC {
    pub fn new() -> Self {
        Self {
            recent_hash_list: VecDeque::new(),
            state: State::Initial,
        }
    }

    // curl -sSL "https://mempool.space/api/blocks/tip/hash"
    // 0000000000000000000624d76f52661d0f35a0da8b93a87cb93cf08fd9140209
    pub async fn start<'a, R>(&mut self, runtime: &'a R) 
    where R: Runtime {
        self.turn(State::Running);
        while let State::Running = self.state {
            info!("poll for new block hash");
            if let Err(e) = self.update_latest_hash(runtime).await {
                warn!("failed to update latest hash: {:?}", e);
            }
            sleep(Duration::from_secs(10)).await;
        }
    }

    fn turn(&mut self, state: State) {
        self.state = state;
    }

    async fn update_latest_hash<'a, R>(&mut self, runtime: &'a R) -> Result<(), Status>
    where R: Runtime {
        debug!("fetching latest block hash from mempool.space");
        let response = runtime.http_call(
            "mempool",
            vec![
                (":method", "GET"),
                (":path", "/api/blocks/tip/hash"),
                (":authority", "mempool.space"),
                (":schema", "https"),
                ("accept", "application/json"),
            ],
            None,
            vec![],
            Duration::from_secs(1),
        )?
        .await
        .map_err(|_| Status::InternalFailure)?;
        
        debug!("receive mempool.space response");

        let Some(body) = response.body else {
            warn!("empty response body");
            return Err(Status::InternalFailure);
        };

        let body_str = match String::from_utf8(body) {
            Ok(body_str) => body_str,
            Err(e) => {
                warn!("invalid response body: {}", e);
                return Err(Status::InternalFailure);
            }
        };

        debug!("response body: {}", body_str);
        if self.recent_hash_list.contains(&body_str) {
            return Ok(());
        }

        info!("New block hash: {}", body_str);

        self.recent_hash_list.push_front(body_str);

        if self.recent_hash_list.len() > 2 {
            let _: Vec<_> = self.recent_hash_list.drain(2..).collect();
        }

        Ok(())
    }

    fn stop(&mut self) {
        self.turn(State::Stopped);
    }
}
