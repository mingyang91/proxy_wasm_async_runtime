use core::panic;
use std::{
    cell::RefCell, collections::HashMap, future::Future, pin::Pin, rc::Rc, task::{Poll, Waker}, time::Duration
};

use log::{info, warn};
use proxy_wasm::{
    hostcalls, traits::{Context, HttpContext, RootContext}, types::{Action, Status}
};

use crate::runtime;

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
        PENDINGS.with(|pendings| pendings.insert(token, promise.clone()));
        Ok(promise)
    }

    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        true
    }
}

pub struct RuntimeBox<R: Runtime> {
    inner: R
}

impl <R: Runtime> RuntimeBox<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner
        }
    }
}

impl <R: Runtime> Context for RuntimeBox<R> {
    fn on_http_call_response(
        &mut self,
        token_id: u32,
        num_headers: usize,
        body_size: usize,
        _num_trailers: usize,
    ) {
        if let Some(promise) = PENDINGS.with(|pendings| pendings.remove(&token_id)) {
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

struct Pendings {
    inner: RefCell<HashMap<u32, Promise>>,
}

impl Pendings {
    fn new() -> Self {
        Self {
            inner: RefCell::new(HashMap::new()),
        }
    }

    fn insert(&self, token: u32, promise: Promise) {
        if let Some(_) = self.inner.borrow_mut().insert(token, promise) {
            panic!("overwriting pending promise for token: {}", token);
        }
    }

    fn remove(&self, token: &u32) -> Option<Promise> {
        self.inner.borrow_mut().remove(token)
    }
}

thread_local! {
    pub(crate) static PENDINGS: Pendings = Pendings::new();
}

impl <R: Runtime> RootContext for RuntimeBox<R> {
    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        self.set_tick_period(Duration::from_millis(1));
        self.inner.on_vm_start(_vm_configuration_size)
    }

    fn on_tick(&mut self) {
        runtime::queue::QUEUE.with(|queue| queue.on_tick());
    }
}

pub struct Ctx { id: u32 }

impl Context for Ctx {}

impl HttpContext for Ctx {}

impl Ctx {
    pub fn new(id: u32) -> Self {
        Self { id }
    }
    pub fn get_http_request_headers(&self) -> Vec<(String, String)> {
        hostcalls::set_effective_context(self.id).expect("failed to set effective context");
        HttpContext::get_http_request_headers(self)
    }
}

pub trait HttpHook {
    fn on_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> impl Future<Output = Result<(), Response>> + Send;
}

pub struct HookHolder<H: HttpHook + 'static> {
    context_id: u32,
    inner: Rc<RefCell<H>>,
}

impl <H: HttpHook + From<u32> + 'static> HookHolder<H> {
    pub fn new(context_id: u32) -> Self {
        info!("new http context: {}", context_id);
        Self {
            context_id,
            inner: Rc::new(RefCell::new(context_id.into())),
        }
    }
}

impl <H: HttpHook> Context for HookHolder<H> {}

impl <H: HttpHook> HttpContext for HookHolder<H> {
    fn on_http_request_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        info!("on_http_request_headers");
        let hook = self.inner.clone();
        let context_id = self.context_id;
        spawn_local(async move {
            hostcalls::set_effective_context(context_id).expect("failed to set effective context");
            let res = hook.borrow_mut().on_request_headers(_num_headers, _end_of_stream).await;
            hostcalls::set_effective_context(context_id).expect("failed to set effective context");
            let ret = match res {
                Ok(()) => { 
                    info!("resume http request: {}", context_id);
                    hostcalls::resume_http_request() 
                },
                Err(resp) => {
                    let headers: Vec<(&str, &str)> = resp.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    info!("reject http request");
                    hostcalls::send_http_response(400, headers, resp.body.as_deref())
                },
            };
            if let Err(e) = ret {
                warn!("failed to resume http request: {:?}", e);
            }
        });
        Action::Pause
    }

    fn on_http_response_headers(&mut self, _num_headers: usize, _end_of_stream: bool) -> Action {
        info!("on_http_response_headers");
        self.set_http_response_header("X-Filter-Name", Some("PoW"));
        Action::Continue
    }
}
