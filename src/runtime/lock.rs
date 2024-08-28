use std::any::type_name;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use proxy_wasm::hostcalls;
use proxy_wasm::types::Status;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueId(pub u32);

/// retister queue per lock key, return queue id
/// wake TryLock when queue data is ready
struct QueueMap {
    tasks: RefCell<HashMap<QueueId, VecDeque<Waker>>>,
}

impl QueueMap {
    fn new() -> Self {
        QueueMap {
            tasks: RefCell::new(HashMap::new()),
        }
    }

    fn push_task(&self, queue_id: QueueId, waker: Waker) {
        let mut tasks = self.tasks.borrow_mut();
        if let Some(wakers) = tasks.get_mut(&queue_id) {
            wakers.push_back(waker);
        } else {
            tasks.insert(queue_id, VecDeque::from(vec![waker]));
        }
    }

    fn wake_tasks(&self, queue_id: QueueId) {
        let mut tasks = self.tasks.borrow_mut();
        if let Some(wakers) = tasks.get_mut(&queue_id) {
            for waker in wakers.drain(..) {
                waker.wake();
            }
        }
    }
}

fn push_task(queue_id: QueueId, waker: Waker) {
    QUEUE_MAP.with(|queue_map| {
        queue_map.push_task(queue_id, waker);
    });
}

pub(crate) fn wake_tasks(queue_id: QueueId) {
    QUEUE_MAP.with(|queue_map| {
        queue_map.wake_tasks(queue_id);
    });
}

thread_local! {
    pub(crate) static QUEUE_MAP: QueueMap = QueueMap::new();
}

#[derive(Debug, Serialize, Deserialize)]
struct Store<T> {
    state: StoreState,
    data: T
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum StoreState {
    Unlocked,
    Locked {
        holder: u32,
        time: u64,
        cas: u32,
    },
}

impl <T> Store<T> {
    fn new(data: T) -> Self {
        Store {
            state: StoreState::Unlocked,
            data,
        }
    }

    fn turn_lock(&mut self, holder: u32, cas: u32) {
        self.state = StoreState::Locked {
            holder,
            time: current_timestamp(),
            cas,
        }
    }

    fn turn_unlock(&mut self) {
        self.state = StoreState::Unlocked;
    }

    fn is_locked(&self) -> bool {
        matches!(self.state, StoreState::Locked { .. })
    }
}


#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("status error({status:?}): {reason}")]
    Status {
        reason: String,
        status: proxy_wasm::types::Status,
    },

    #[error("CAS mismatch")]
    CasMismatch,

    #[error("shared data is locked")]
    Locked,

    #[error("failed to decode shared data: {0}")]
    Decode(#[from] serde_json::Error),
}

impl Error {
    fn status(reason: String, status: proxy_wasm::types::Status) -> Self {
        Error::Status { reason, status }
    }
}

/// A structure that represents a lock on shared data.
///
/// This struct holds a reference to a piece of shared data along with
/// a unique key and a compare-and-swap (CAS) version counter to manage
/// concurrent access to the data.
///
/// # Type Parameters
///
/// * `S` - The type of the shared data that this lock protects.
pub struct SharedDataLock<S> {
    context_id: u32,
    queue_id: QueueId,
    /// A unique key associated with the shared data type.
    key: &'static str,
    /// A counter used for managing concurrency, following the
    /// compare-and-swap (CAS) model.
    cas: u32,
    _phantom: PhantomData<S>,
}

/// A guard that provides temporary access to the shared data
/// protected by a `SharedDataLock`.
///
/// The lock is released when this guard is dropped, ensuring
/// that the shared data is safely accessible while the guard
/// is in scope.
pub struct SharedDataLockGuard<'a, S> 
where 
    S: Serialize + DeserializeOwned 
{
    lock: &'a SharedDataLock<S>,
    store: Store<S>,
}

impl<'a, S> SharedDataLockGuard<'a, S> 
where 
    S: Serialize + DeserializeOwned 
{
    fn new(lock: &'a SharedDataLock<S>, store: Store<S>) -> Self {
        SharedDataLockGuard {
            lock,
            store,
        }
    }
}

impl <S> Drop for SharedDataLockGuard<'_, S> 
where
    S: Serialize + DeserializeOwned
{
    fn drop(&mut self) {
        set_and_unlock_shared_data(self.lock.key, self.lock.queue_id, &mut self.store)
            .expect("failed to unlock shared data");
    }
}

impl <S> Deref for SharedDataLockGuard<'_, S> 
where 
    S: Serialize + DeserializeOwned 
{
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.store.data
    }
}

impl <S> DerefMut for SharedDataLockGuard<'_, S> 
where 
    S: Serialize + DeserializeOwned 
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.store.data
    }
}

impl<S: 'static> SharedDataLock<S> {
    /// Create a new lock for the given shared data.
    pub fn new(context_id: u32) -> Self {
        let key = type_name::<S>();
        let queue_id = QueueId(hostcalls::register_shared_queue(key)
            .expect("failed to register shared queue"));
        SharedDataLock {
            context_id,
            queue_id,
            key,
            cas: 0,
            _phantom: PhantomData,
        }
    }
    
    pub fn initial(&self, data: S) -> Result<(), Error>
    where
        S: Serialize + DeserializeOwned
    {
        let store = Store::new(data);
        let raw = serde_json::to_vec(&store)
            .expect("failed to serialize shared data");

        match hostcalls::set_shared_data(self.key, Some(&raw), None) {
            Ok(_) => Ok(()),
            Err(Status::CasMismatch) => Err(Error::CasMismatch),
            Err(status) => Err(Error::status("failed to set shared data".to_string(), status)),
        }
    }

    /// Acquire a lock on the shared data.
    pub fn lock(&self) -> TryLock<S> {
        TryLock { lock: self, gone: false }
    }
}



pub struct TryLock<'a, S> {
    lock: &'a SharedDataLock<S>,
    gone: bool,
}

impl<'a, S> Future for TryLock<'a, S> 
where 
    S: Serialize + DeserializeOwned + Debug
{
    type Output = Result<SharedDataLockGuard<'a, S>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.gone {
            panic!("polling a resolved promise");
        }

        let res = get_and_lock_shared_data(this.lock.key, this.lock.context_id); // todo: change me
        match res {
            Ok(store) => {
                this.gone = true;
                Poll::Ready(Ok(SharedDataLockGuard::new(this.lock, store)))
            }
            Err(Error::CasMismatch | Error::Locked) => {
                push_task(this.lock.queue_id, cx.waker().clone());
                Poll::Pending
            }
            Err(err) => {
                this.gone = true;
                Poll::Ready(Err(err))
            }
        }
    }
}

fn get_shared_data<T: DeserializeOwned>(key: &str) -> Result<(Option<T>, Option<u32>), Error> {
    let (raw, cas) = hostcalls::get_shared_data(key)
        .map_err(|status| Error::status("failed to get shared data".to_string(), status))?;

    match raw {
        None => Ok((None, cas)),
        Some(vec) => {
            let data = serde_json::from_slice(&vec)?;
            Ok((Some(data), cas))
        }
    }
}

fn get_and_lock_shared_data<T>(key: &str, holder: u32) -> Result<Store<T>, Error> 
where 
    T: Serialize + DeserializeOwned + Debug
{
    let (raw, cas) = hostcalls::get_shared_data(key)
        .map_err(|status| Error::status("failed to get shared data".to_string(), status))?;

    let Some(cas) = cas else {
        return Err(Error::Status { // TODO: changeme
            reason: "missing CAS value".to_string(),
            status: proxy_wasm::types::Status::BadArgument,
        });
    };

    let Some(vec) = raw else {
        return Err(Error::Status { // TODO: changeme
            reason: "shared data is null".to_string(),
            status: proxy_wasm::types::Status::Empty,
        });
    };

    let mut store: Store<T> = serde_json::from_slice(&vec)?;

    if store.is_locked() {
        return Err(Error::Locked);
    }

    store.turn_lock(holder, cas);
    let raw = serde_json::to_vec(&store)?;
    let Err(status) = hostcalls::set_shared_data(key, Some(&raw), Some(cas)) else {
        return Ok(store)
    };

    let err = match status {
        proxy_wasm::types::Status::CasMismatch => Error::CasMismatch,
        _ => Error::status("failed to set shared data".to_string(), status),
    };
    Err(err)
}

fn set_and_unlock_shared_data<T>(key: &str, queue_id: QueueId, store: &mut Store<T>) -> Result<(), Error> 
where 
    T: Serialize + DeserializeOwned {
    let cas = {
        let StoreState::Locked { holder: _, time: _, cas } = &store.state else {
            log::error!("???");
            return Ok(())
        };
        *cas
    };

    store.turn_unlock();
    let raw = serde_json::to_vec(&store)?;

    let Err(status) = hostcalls::set_shared_data(key, Some(&raw), Some(cas + 1)) else {
        hostcalls::enqueue_shared_queue(queue_id.0, None) // TODO: change me
            .map_err(|status| Error::status("failed to enqueue shared queue".to_string(), status))?;
        return Ok(())
    };

    let err = match status {
        proxy_wasm::types::Status::CasMismatch => Error::CasMismatch,
        _ => Error::status("failed to set shared data".to_string(), status),
    };
    Err(err)
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .expect("failed to get timestamp")
        .as_secs()
}

#[cfg(test)]
mod test {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    struct Wukong {
        name: String
    }
    
    #[test]
    fn test_shared_data_lock() {
        let json = "{\"state\":{\"type\":\"Unlocked\"},\"data\":{\"name\":\"Sun\"}}";
        let data: Store<Wukong> = serde_json::from_str(json).expect("failed to deserialize shared data");
    }
}