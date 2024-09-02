use std::{collections::VecDeque, time::Duration};
use std::sync::RwLock;

use log::{debug, warn};
use proxy_wasm::types::Status;
use serde::{Deserialize, Serialize};

use crate::runtime::lock::SharedDataLock;
use crate::runtime::{timeout::sleep, Runtime};

pub struct BTC {
    recent_hash_list: SharedDataLock<VecDeque<String>>,
    state: RwLock<State>,
}

impl Default for BTC {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Initial,
    Running,
    Stopped,
}

impl BTC {
    pub fn new() -> Self {
        let recent_hash_list = SharedDataLock::new(0);
        if let Err(e) = recent_hash_list.initial(VecDeque::new()) {
            log::info!("failed to initialize shared data: {:?}", e);
        }
        Self {
            recent_hash_list,
            state: RwLock::new(State::Initial),
        }
    }

    pub fn get_latest_hash(&self) -> Option<String> {
        self.recent_hash_list
            .read()
            .expect("failed to read recent hash list")
            .front()
            .cloned()
    }

    // curl -sSL "https://mempool.space/api/blocks/tip/hash"
    // 0000000000000000000624d76f52661d0f35a0da8b93a87cb93cf08fd9140209
    pub async fn start<'a, R>(&self, runtime: &'a R) 
    where R: Runtime {
        self.turn(State::Running);
        loop {
            { 
                let state = *self.state.read().expect("failed to read state");
                if State::Running != state { 
                    log::info!("exit polling loop");
                    break; 
                }
            }
            log::debug!("poll for new block hash");
            if let Err(e) = self.update_latest_hash(runtime).await {
                warn!("failed to update latest hash: {:?}", e);
            }

            let lock = self.recent_hash_list.lock().await
                .expect("failed to acquire lock");
            sleep(Duration::from_secs(10)).await;
            debug!("data: {:?}", *lock);
        }
    }

    fn turn(&self, state: State) {
        *self.state.write().expect("failed to write state") = state;
    }

    async fn update_latest_hash<'a, R>(&self, runtime: &'a R) -> Result<(), Status>
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

        let body_str = String::from_utf8(body)
            .map_err(|e| {
                warn!("invalid response body: {}", e);
                Status::InternalFailure
            })?;

        let mut recent_hash_list = self.recent_hash_list.lock().await.expect("failed to write recent hash list");
        debug!("response body: {}", body_str);
        if recent_hash_list.contains(&body_str) {
            return Ok(());
        }

        debug!("New block hash: {}", body_str);

        recent_hash_list.push_front(body_str);

        if recent_hash_list.len() > 2 {
            let _: Vec<_> = recent_hash_list.drain(2..).collect();
        }

        Ok(())
    }

    pub fn stop(&mut self) {
        self.turn(State::Stopped);
    }
}
