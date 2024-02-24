FROM --platform=$BUILDPLATFORM rust:1.76 AS builder
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
      "linux/arm/v7") echo armv7-unknown-linux-musleabihf > /rust_target.txt ;; \
      "linux/arm64") echo aarch64-unknown-linux-musl > /rust_target.txt ;; \
      "linux/amd64") echo x86_64-unknown-linux-musl > /rust_target.txt ;; \
      *) exit 1 ;; \
    esac
RUN rustup target add $(cat /rust_target.txt)

WORKDIR /app
COPY . .
RUN cargo build --target $(cat /rust_target.txt) --release --bins
RUN mv /app/target/$(cat /rust_target.txt)/release/rust-mdns-repeater /app/rust-mdns-repeater

FROM alpine:latest as release
WORKDIR /app
COPY --from=builder /app/rust-mdns-repeater /app/rust-mdns-repeater
ENTRYPOINT [ "/app/rust-mdns-repeater" ]
CMD [ "--help" ]