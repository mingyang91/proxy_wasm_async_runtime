FROM nginx:latest
COPY target/wasm32-wasip1/release/pow_waf.wasm /usr/share/nginx/html/pow_waf.wasm
