use std::{cell::RefCell, collections::HashMap, future::Future, pin::Pin, rc::Rc, task::{Poll, Waker}};

use super::response::Response;


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
	pub fn pending() -> Self {
			Self {
					inner: Rc::new(RefCell::new(InnerPromise::Pending(None))),
			}
	}

	pub fn resolve(&self, response: Response) {
			let old = self.inner.replace(InnerPromise::Resolved(response));
			if let InnerPromise::Pending(Some(waker)) = old {
					waker.wake();
			}
	}

	pub fn reject(&self) {
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
					Poll::Pending
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

pub struct Pendings {
	inner: RefCell<HashMap<u32, Promise>>,
}

impl Pendings {
	pub(crate) fn new() -> Self {
			Self {
					inner: RefCell::new(HashMap::new()),
			}
	}

	pub(crate) fn insert(&self, token: u32, promise: Promise) {
			if self.inner.borrow_mut().insert(token, promise).is_some() {
					panic!("overwriting pending promise for token: {}", token);
			}
	}

	pub(crate) fn remove(&self, token: &u32) -> Option<Promise> {
			self.inner.borrow_mut().remove(token)
	}
}

thread_local! {
	pub(crate) static PENDINGS: Pendings = Pendings::new();
}
