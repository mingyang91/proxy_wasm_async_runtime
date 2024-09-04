use std::{collections::VecDeque, marker::PhantomData, time::Duration};

use proxy_wasm::{hostcalls, types::Status};
use serde::{Deserialize, Serialize};

use super::codec::Codec;

pub struct LowLevelKVStore {
    context_id: u32,
}

impl LowLevelKVStore {
    pub fn new(context_id: u32) -> Self {
        Self { 
            context_id,
        }
    }

    pub fn put(&self, key: &str, value: &[u8]) -> Result<(), Status> {
        hostcalls::set_effective_context(self.context_id)?;
        hostcalls::set_shared_data(key, Some(value), None)?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, Status> {
        hostcalls::set_effective_context(self.context_id)?;
        let (value, _) = hostcalls::get_shared_data(key)?;
        Ok(value)
    }

    pub fn remove(&self, key: &str) -> Result<(), Status> {
        hostcalls::set_effective_context(self.context_id)?;
        loop {
            let (value, cas) = hostcalls::get_shared_data(key)?;
            if value.is_none() {
                return Ok(());
            }
            match hostcalls::set_shared_data(key, None, cas) {
                Ok(()) => return Ok(()),
                Err(Status::CasMismatch) => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn update<F>(&self, key: &str, mut f: F) -> Result<Vec<u8>, Status>
    where
        F: FnMut(Option<Vec<u8>>) -> Vec<u8>,
    {
        hostcalls::set_effective_context(self.context_id)?;
        loop {
            let (value, cas) = hostcalls::get_shared_data(key)?;
            let new_value = f(value);
            match hostcalls::set_shared_data(key, Some(&new_value), cas) {
                Ok(()) => return Ok(new_value),
                Err(Status::CasMismatch) => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

pub struct KVStore<V> {
    low_level: LowLevelKVStore,
    prefix: String,
    _phantom: PhantomData<V>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Status: [{status:?}]: {description}")]
    Status {
        status: Status,
        description: String,
    },
    #[error("Failed to decode/encode value: {0}")]
    Codec(#[from] Box<dyn std::error::Error>),
}

impl Error {
    pub fn status(status: Status, description: impl Into<String>) -> Self {
        Self::Status {
            status,
            description: description.into(),
        }
    }
}

impl <V: Codec> KVStore<V>
where 
    V::Error: Into<Box<dyn std::error::Error>>
{
    pub fn new(context_id: u32, prefix: &str) -> Self {
        Self {
            low_level: LowLevelKVStore::new(context_id),
            prefix: prefix.to_string(),
            _phantom: PhantomData,
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<V>, Error> {
        let value = self.low_level
            .get(&format!("{}{}", self.prefix, key))
            .map_err(|s| Error::status(s, "failed to get value"))?;

        match value {
            Some(v) => Ok(Some(
                V::decode(&v).map_err(|e| Error::Codec(e.into()))?
            )),
            None => Ok(None),
        }
    }

    pub fn put(&self, key: &str, value: &V) -> Result<(), Error> {
        let encoded = value.encode().map_err(|e| Error::Codec(e.into()))?;
        self.low_level
            .put(&format!("{}{}", self.prefix, key), &encoded)
            .map_err(|s| Error::status(s, "failed to put value"))
    }

    pub fn remove(&self, key: &str) -> Result<(), Error> {
        self.low_level
            .remove(&format!("{}{}", self.prefix, key))
            .map_err(|s| Error::status(s, "failed to remove value"))
    }

    pub fn update<F>(&self, key: &str, mut f: F) -> Result<V, Error>
    where
        F: FnMut(Option<V>) -> V,
    {
        let value = self.low_level
            .update(&format!("{}{}", self.prefix, key), |old_value| {
                let new_value = f(old_value.map(|v| {
                    V::decode(&v).map_err(|e| Error::Codec(e.into())).unwrap()
                }));
                new_value.encode().map_err(|e| Error::Codec(e.into())).unwrap()
            })
            .map_err(|s| Error::status(s, "failed to update value"))?;

        V::decode(&value).map_err(|e| Error::Codec(e.into()))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Expirations {
    list: VecDeque<(u64, String)>,
}

impl Expirations {
    fn new() -> Self {
        Self {
            list: VecDeque::new(),
        }
    }

    fn push(&mut self, key: String, ttl: Duration) {
        let expiration = Self::now() + ttl.as_secs();
        self.list.push_back((expiration, key));
        self.list.make_contiguous().sort();
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn pop_expired(&mut self) -> Vec<String> {
        let now = Self::now();
        let mut expired = Vec::new();
        while let Some((expiration, key)) = self.list.front() {
            if *expiration > now {
                break;
            }
            expired.push(key.clone());
            self.list.pop_front();
        }
        expired
    }
}

pub struct ExpiringKVStore<V> {
    store: KVStore<V>,
    expirations: KVStore<Expirations>
}

impl <V> ExpiringKVStore<V>
where 
    V: Codec,
    V::Error: Into<Box<dyn std::error::Error>>
{
    pub fn new(context_id: u32, prefix: &str) -> Self {
        Self {
            store: KVStore::new(context_id, prefix),
            expirations: KVStore::new(context_id, &format!("{}:expirations", prefix)),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<V>, Error> {
        self.store.get(key)
    }

    pub fn put(&self, key: &str, value: &V, ttl: Duration) -> Result<(), Error> {
        self.store.put(key, value)?;
        self.enqueue_expires(key, ttl)
    }

    pub fn remove(&self, key: &str) -> Result<(), Error> {
        self.store.remove(key)
    }

    pub fn update<F>(&self, key: &str, f: F) -> Result<V, Error>
    where
        F: FnMut(Option<V>) -> V,
    {
        self.store.update(key, f)
    }

    pub fn enqueue_expires(&self, key: &str, ttl: Duration) -> Result<(), Error> {
        let _ = self.expirations.update("", |expirations| {
            let mut expirations = expirations.unwrap_or_else(Expirations::new);
            expirations.push(key.to_string(), ttl);
            expirations
        })?;
        self.gc()
    }

    pub fn gc(&self) -> Result<(), Error> {
        let mut expired = vec![];
        let _ = self.expirations.update("", |expirations| {
            let Some(mut expirations) = expirations else {
                return Expirations::new();
            };
            expired = expirations.pop_expired();
            return expirations
        })?;

        for key in expired {
            let _ = self.store.remove(&key)?;
        }

        Ok(())
    }
}
