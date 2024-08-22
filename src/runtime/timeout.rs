use std::{future::Future, io, pin::Pin, task::{Context, Poll}};
use pin_project_lite::pin_project;
use std::io::Result;

#[derive(Debug)]
pub struct Timer {
    // The time at which the timeout will expire
    expiry: std::time::Instant,
}

impl Timer {
    fn new(duration: std::time::Duration) -> Self {
        Self {
            expiry: std::time::Instant::now() + duration,
        }
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if std::time::Instant::now() >= self.expiry {
            Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

pin_project! {
    /// Future returned by the `FutureExt::timeout` method.
    #[derive(Debug)]
    pub struct Timeout<F, T>
    where
        F: Future<Output = Result<T>>,
    {
        #[pin]
        future: F,
        #[pin]
        timeout: Timer,
    }
}


impl<F, T> Future for Timeout<F, T>
where
    F: Future<Output = Result<T>>,
{
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.future.poll(cx) {
            Poll::Pending => {}
            other => return other,
        }

        if this.timeout.poll(cx).is_ready() {
            let err = Err(io::Error::new(io::ErrorKind::TimedOut, "future timed out"));
            Poll::Ready(err)
        } else {
            Poll::Pending
        }
    }
}

pub fn sleep(duration: std::time::Duration) -> Timer {
    Timer::new(duration)
}

pub fn timeout<F, T>(future: F, duration: std::time::Duration) -> Timeout<F, T>
where
    F: Future<Output = Result<T>>,
{
    Timeout {
        future,
        timeout: Timer::new(duration),
    }
}