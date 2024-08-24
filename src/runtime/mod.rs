use std::{
    cell::RefCell, collections::HashMap, future::Future, ops::DerefMut, pin::Pin, rc::Rc, task::{Poll, Waker}, time::Duration
};

use log::info;
use proxy_wasm::{
    traits::{Context, RootContext},
    types::Status,
};
use timeout::sleep;

use crate::{chain, runtime};

mod task {
    mod singlethread;
    pub(crate) use singlethread::*;
}
pub mod queue;
pub mod timeout;

/// Runs a Rust `Future` on the current thread.
///
/// The `future` must be `'static` because it will be scheduled
/// to run in the background and cannot contain any stack references.
///
/// The `future` will always be run on the next microtask tick even if it
/// immediately returns `Poll::Ready`.
///
/// # Panics
///
/// This function has the same panic behavior as `future_to_promise`.
#[inline]
pub fn spawn_local<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    task::Task::spawn(Box::pin(future));
}

pub struct Response {
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub trailers: Vec<(String, String)>,
}

enum InnerPromise {
    Pending(Option<Waker>),
    Resolved(Response),
    Rejected,
    Gone(()),
}

#[derive(Clone)]
pub struct Promise {
    inner: Rc<RefCell<InnerPromise>>,
}

impl Promise {
    fn pending() -> Self {
        Self {
            inner: Rc::new(RefCell::new(InnerPromise::Pending(None))),
        }
    }

    fn resolve(&self, response: Response) {
        let old = self.inner.replace(InnerPromise::Resolved(response));
        if let InnerPromise::Pending(Some(waker)) = old {
            waker.wake();
        }
    }

    fn reject(&self) {
        self.inner.replace(InnerPromise::Rejected);
    }
}

impl Future for Promise {
    type Output = Result<Response, ()>;

    fn poll(self: Pin<&mut Self>, _cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.inner.borrow_mut();
        if let InnerPromise::Pending(ref mut waker) = *inner {
            if waker.is_none() {
                *waker = Some(_cx.waker().clone());
            }
            return Poll::Pending;
        } else if let InnerPromise::Rejected = *inner {
            return Poll::Ready(Err(()));
        } else if let InnerPromise::Gone(()) = *inner {
            panic!("polling a resolved promise");
        } else {
            match std::mem::replace(&mut *inner, InnerPromise::Gone(())) {
                InnerPromise::Resolved(response) => return Poll::Ready(Ok(response)),
                _ => unreachable!(),
            }
        }
    }
}

pub trait Runtime: Context {
    fn pendings(&self) -> &RefCell<HashMap<u32, Promise>>;

    fn http_call(
        &self,
        upstream: &str,
        headers: Vec<(&str, &str)>,
        body: Option<&[u8]>,
        trailers: Vec<(&str, &str)>,
        timeout: Duration,
    ) -> Result<Promise, Status> {
        let token = Context::dispatch_http_call(self, upstream, headers, body, trailers, timeout)?;
        let promise = Promise::pending();
        self.pendings().borrow_mut().insert(token, promise.clone());
        Ok(promise)
    }
}

#[derive(Default)]
pub struct DefaultRuntimeInner {
    pendings: RefCell<HashMap<u32, Promise>>,
}

#[derive(Default, Clone)]
pub struct DefaultRuntime {
    inner: Rc<DefaultRuntimeInner>,
}

impl Context for DefaultRuntime {
    fn on_http_call_response(
        &mut self,
        token_id: u32,
        num_headers: usize,
        body_size: usize,
        _num_trailers: usize,
    ) {
        if let Some(promise) = self.inner.pendings.borrow_mut().remove(&token_id) {
            if num_headers == 0 {
                promise.reject();
                return;
            }
            let headers = self.get_http_call_response_headers();
            let body = self.get_http_call_response_body(0, body_size);
            let trailers = self.get_http_call_response_trailers();
            let response = Response {
                headers,
                body,
                trailers,
            };
            promise.resolve(response);
        }
    }
}

impl RootContext for DefaultRuntime {
    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        let mut btc = chain::btc::BTC::new();
        info!("Hello from WASM");
        self.set_tick_period(Duration::from_millis(1));
        runtime::spawn_local(async move {
            loop {
                sleep(Duration::from_secs(10)).await;
                info!("beats");
            }
        });
        let runtime = self.clone();
        runtime::spawn_local(async move {
            btc.start(&runtime).await;
        });
        true
    }

    fn on_tick(&mut self) {
        runtime::queue::QUEUE.with(|queue| queue.on_tick());
    }
}

impl Runtime for DefaultRuntime {
    fn pendings(&self) -> &RefCell<HashMap<u32, Promise>> { &self.inner.pendings }
}
