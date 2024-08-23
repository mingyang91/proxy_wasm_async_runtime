use std::{error::Error as StdError, future::Future, pin::Pin, task::{Context, Poll}};
use tokio::io::{AsyncRead, AsyncWrite};
use http::{Request, Uri};
use hyper::client::conn::http1::handshake;
use tower_service::Service;

struct Error;

impl Into<Box<dyn StdError + Send + Sync>> for Error {
		fn into(self) -> Box<dyn StdError + Send + Sync> {
				unimplemented!()
		}
}

struct HostCall;
impl Connection for HostCall {
		fn connected(&self) -> hyper::client::connect::Connected {
				todo!()
		}
}

impl AsyncRead for HostCall {
		fn poll_read(
				self: Pin<&mut Self>,
				cx: &mut Context<'_>,
				buf: &mut tokio::io::ReadBuf<'_>,
		) -> Poll<std::io::Result<()>> {
				todo!()
		}
}

impl AsyncWrite for HostCall {
		fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
				unimplemented!()
		}
		fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
				unimplemented!()
		}
		
		fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
				todo!()
		}
}

struct Pending;
impl Future for Pending {
		type Output = Result<HostCall, Error>;
		fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
				unimplemented!()
		}
}

#[derive(Clone)]
struct PWC;
impl Service<Uri> for PWC {
		type Response = HostCall;
		type Error = Error;
    type Future = Pending;

		fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
				unimplemented!()
		}
		fn call(&mut self, req: Uri) -> Self::Future {
				unimplemented!()
		}
}

async fn test() {
	let conn = handshake().await;
	let client: Client<PWC, String> = hyper::Client::builder().build(PWC);
	let mut res = client.request(
		Request::builder()
			.uri("http://httpbin.org/bytes/1")
			.body("".to_string())
			.unwrap()
	).await.expect("msg");
	let body = res.body_mut();
	body.data().await;
	// let builder = hyper::Client::builder();
	// builder.build();
}