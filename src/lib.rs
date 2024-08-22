pub mod runtime;
// pub mod calloop;

use log::info;
use runtime::timeout::sleep;
use proxy_wasm::traits::*;
use proxy_wasm::types::*;
use std::time::Duration;

proxy_wasm::main! {{
    proxy_wasm::set_log_level(LogLevel::Trace);
    proxy_wasm::set_http_context(|_, _| -> Box<dyn HttpContext> { Box::new(HttpAuthRandom) });
    proxy_wasm::set_root_context(|_| -> Box<dyn RootContext> { Box::new(HttpAuthRandom) });
}}

struct HttpAuthRandom;

impl HttpContext for HttpAuthRandom {
    fn on_http_request_headers(&mut self, _: usize, _: bool) -> Action {
        self.dispatch_http_call(
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
        Action::Pause
    }

    fn on_http_response_headers(&mut self, _: usize, _: bool) -> Action {
        self.set_http_response_header("Powered-By", Some("proxy-wasm"));
        Action::Continue
    }
}

impl Context for HttpAuthRandom {
    fn on_http_call_response(&mut self, _: u32, _: usize, body_size: usize, _: usize) {
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

impl RootContext for HttpAuthRandom {
    fn on_vm_start(&mut self, _vm_configuration_size: usize) -> bool {
        info!("Hello from WASM");
        self.set_tick_period(Duration::from_millis(1));
        runtime::spawn_local(async {
            loop {
                sleep(Duration::from_secs(1)).await;
                info!("beats");
            }
        });
        true
    }

    fn on_tick(&mut self) {
        runtime::queue::QUEUE.with(|queue| {
            queue.on_tick();
        });
    }
}