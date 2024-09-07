FROM rust:bullseye AS builder
RUN rustup target add wasm32-wasi
WORKDIR /usr/src/build
COPY . .
RUN cargo build --release --target wasm32-wasi


FROM scratch AS runtime
COPY --from=builder /usr/src/build/target/wasm32-wasi/release/pow_waf.wasm ./