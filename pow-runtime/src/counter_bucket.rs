use std::{collections::HashMap, sync::{Arc, Mutex}, time::Duration};

use thiserror::Error;

use super::{kv_store::ExpiringKVStore, spawn_local, timeout::sleep};


#[derive(Clone)]
pub struct CounterBucket {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    pub store: ExpiringKVStore<u64>,
    pub buffer: HashMap<String, u64>,
    pub stop: bool,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to read/write value: {0}")]
    KV(#[from] super::kv_store::Error),
}

impl Drop for CounterBucket {
    fn drop(&mut self) {
        let mut lock = self.inner.lock().expect("failed to lock inner");
        lock.stop = true;
    }
}

impl CounterBucket {
    pub fn new(context_id: u32, prefix: &str) -> Self {
        let ret = Self {
            inner: Arc::new(Mutex::new(Inner {
                store: ExpiringKVStore::new(context_id, prefix),
                buffer: HashMap::new(),
                stop: false,
            }))
        };
        let ret_clone = ret.clone();
        spawn_local(async move {
            ret_clone.background_task().await
        });
        ret
    }

    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone()
        }
    }

    pub fn inc(&self, key: &str, value: u64) {
        let mut inner = self.inner.lock().expect("failed to lock inner");
        let counter = inner.buffer.entry(key.to_string()).or_insert(0);
        *counter += value;
    }

    pub fn get(&self, key: &str) -> Result<u64, Error> {
        let inner = self.inner.lock().expect("failed to lock inner");
        let counter = inner.store.get(key)?.unwrap_or(0);
        let delta = inner.buffer.get(key).copied().unwrap_or(0);
        Ok(counter + delta)
    }

    pub fn flush(&self) -> usize {
        let mut inner = self.inner.lock().expect("failed to lock inner");
        let buffer: Vec<(String, u64)> = inner.buffer.drain().collect();
        let len = buffer.len();
        for (key, value) in buffer {
            let _ = inner.store.update(&key, |old| old.unwrap_or(0) + value);
        }
        len
    }

    pub async fn background_task(&self) {
        loop {
            sleep(Duration::from_secs(1)).await;
            let _flushed = self.flush();
            if self.inner.lock().expect("failed to lock inner").stop {
                break;
            }
        }
    }
}
