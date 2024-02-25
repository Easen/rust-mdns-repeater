FROM rust:1.76 AS builder
ARG TARGETPLATFORM
ARG TARGETARCH
RUN case "$TARGETPLATFORM" in \
      "linux/arm/v7") echo armv7-unknown-linux-musleabihf > /$TARGETARCH.txt ;; \
      "linux/arm64") echo aarch64-unknown-linux-musl > /$TARGETARCH.txt ;; \
      "linux/amd64") echo x86_64-unknown-linux-musl > /$TARGETARCH.txt ;; \
      *) exit 1 ;; \
    esac
RUN rustup target add $(cat /$TARGETARCH.txt)
WORKDIR /app
COPY . .
RUN cargo build --target $(cat /$TARGETARCH.txt) --release --bins
RUN mv /app/target/$(cat /$TARGETARCH.txt)/release/rust-mdns-repeater /app/rust-mdns-repeater

FROM alpine:latest as release
WORKDIR /app
COPY --from=builder /app/rust-mdns-repeater /app/rust-mdns-repeater
ENTRYPOINT [ "/app/rust-mdns-repeater" ]
CMD [ "--help" ]