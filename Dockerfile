FROM rust:1.76 as builder
WORKDIR /app
ARG TARGET="aarch64-unknown-linux-musl"
RUN rustup target add $TARGET
COPY . .
RUN cargo build --target $TARGET --release --bins
RUN mv /app/target/$TARGET/release/rust-mdns-repeater /app/rust-mdns-repeater

FROM alpine:latest as release
WORKDIR /app
COPY --from=builder /app/rust-mdns-repeater /app/rust-mdns-repeater
ENTRYPOINT [ "/app/rust-mdns-repeater" ]
CMD [ "--help" ]