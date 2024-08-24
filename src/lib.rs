pub mod runtime;
pub mod chain;

use log::info;
use proxy_wasm::traits::*;
use proxy_wasm::types::*;
use runtime::DefaultRuntime;
use std::time::Duration;

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);
    proxy_wasm::set_http_context(|_, _| -> Box<dyn HttpContext> { Box::new(HttpAuthRandom::default()) });
    proxy_wasm::set_root_context(|_| -> Box<dyn RootContext> { Box::new(DefaultRuntime::default()) });
}}

#[derive(Default)]
struct HttpAuthRandom { token: Option<u32> }

impl HttpContext for HttpAuthRandom {
    fn on_http_request_headers(&mut self, _: usize, _: bool) -> Action {
        let token = self.dispatch_http_call(
            "httpbin",
            vec![
                (":method", "GET"),
                (":path", "/bytes/1"),
                (":authority", "httpbin.org"),
            ],
            None,
            vec![],
            Duration::from_secs(1),
        )
        .unwrap();
        self.token.replace(token);
        Action::Pause
    }

    fn on_http_response_headers(&mut self, _: usize, _: bool) -> Action {
        self.set_http_response_header("Powered-By", Some("proxy-wasm"));
        Action::Continue
    }
}

impl Context for HttpAuthRandom {
    fn on_http_call_response(&mut self, token: u32, _: usize, body_size: usize, _: usize) {
        if Some(token) != self.token {
            return;
        }
        if let Some(body) = self.get_http_call_response_body(0, body_size) {
            if !body.is_empty() && body[0] % 2 == 0 {
                info!("Access granted.");
                self.resume_http_request();
                return;
            }
        }
        info!("Access forbidden.");
        self.send_http_response(
            403,
            vec![("Powered-By", "proxy-wasm")],
            Some(b"Access forbidden.\n"),
        );
    }
}
