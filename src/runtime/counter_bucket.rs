use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use thiserror::Error;

use super::{kv_store::ExpiringKVStore, spawn_local, timeout::sleep};


pub struct CounterBucket {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    pub store: ExpiringKVStore<u64>,
    pub buffer: HashMap<String, u64>,
    pub stop: bool,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get value: {0}")]
    Get(#[from] super::kv_store::Error),
}

impl Drop for CounterBucket {
    fn drop(&mut self) {
        self.inner.borrow_mut().stop = true;
    }
}

impl CounterBucket {
    pub fn new(context_id: u32, prefix: &str) -> Self {
        let ret = Self {
            inner: Rc::new(RefCell::new(Inner {
                store: ExpiringKVStore::new(context_id, prefix),
                buffer: HashMap::new(),
                stop: false,
            }))
        };
        let ret_clone = ret.clone();
        spawn_local(async move {
            ret_clone.background_task().await;
        });
        ret
    }

    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone()
        }
    }

    pub fn inc(&self, key: &str, value: u64) {
        let mut inner = self.inner.borrow_mut();
        let counter = inner.buffer.entry(key.to_string()).or_insert(0);
        *counter += value;
    }

    pub fn get(&self, key: &str) -> Result<u64, Error> {
        let inner = self.inner.borrow();
        let counter = inner.store.get(key)?.unwrap_or(0);
        let delta = inner.buffer.get(key).copied().unwrap_or(0);
        Ok(counter + delta)
    }

    pub fn flush(&self) {
        let mut inner = self.inner.borrow_mut();
        let buffer: Vec<(String, u64)> = inner.buffer.drain().collect();
        for (key, value) in buffer {
            let _ = inner.store.update(&key, |old| old.unwrap_or(0) + value);
        }
    }

    pub async fn background_task(&self) {
        while !self.inner.borrow().stop {
            sleep(Duration::from_secs(1)).await;
            self.flush();
        }
    }
}
