use bytes::Bytes;
use http::{Request, Uri};
use http_body::Body;
use hyper::{
    body::Incoming, client::conn::http1::{handshake, Connection}, rt::ReadBufCursor
};
use pin_project_lite::pin_project;
use std::{
    collections::VecDeque, error::Error as StdError, future::Future, pin::{pin, Pin}, task::{Context, Poll}
};
use tokio::io::{AsyncRead, AsyncWrite};
use tower_service::Service;

struct Error;

impl Into<Box<dyn StdError + Send + Sync>> for Error {
    fn into(self) -> Box<dyn StdError + Send + Sync> {
        unimplemented!()
    }
}

struct HostCall;
impl hyper::rt::Read for HostCall {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        todo!()
    }
}

impl hyper::rt::Write for HostCall {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        unimplemented!()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        unimplemented!()
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        todo!()
    }
}

struct Entity;
impl http_body::Body for Entity {
    type Data = VecDeque<u8>;
    type Error = Error;
    
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        todo!()
    }
}

pin_project! {
    struct Pending {
        #[pin]
        incoming: Incoming,
    }
}

impl Future for Pending {
    type Output = Result<Bytes, Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project().incoming.poll_frame(cx) {
            Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Ok(chunk.into_data().expect("msg"))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(Error)),
            Poll::Ready(None) => Poll::Ready(Err(Error)),
            Poll::Pending => Poll::Pending,
        }
    }
}

async fn test() {
    let (mut send_request, connection) = handshake(HostCall).await
        .expect("msg");
    let mut res = send_request.send_request(
            Request::builder()
                .uri("http://httpbin.org/bytes/1")
                .body(Entity)
                .unwrap()
        )
        .await
        .expect("msg");
    let (header, body) = res.into_parts();
    // let builder = hyper::Client::builder();
    // builder.build();
}

