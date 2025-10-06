FROM rust:1.90 AS builder

RUN apt update && \
    apt install -y musl-tools musl-dev && \
    update-ca-certificates

ENV USER=user
ENV UID=10001

RUN adduser \
--disabled-password \
--gecos "" \
--home "/nonexistent" \
--shell "/sbin/nologin" \
--no-create-home \
--uid "${UID}" \
"${USER}"

WORKDIR /app
COPY . .
ENV RUST_TARGET=""
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
    "linux/amd64") export RUST_TARGET=x86_64-unknown-linux-musl ;; \
    "linux/arm64") export RUST_TARGET=aarch64-unknown-linux-musl ;; \
    "linux/arm/v7") export RUST_TARGET=armv7-unknown-linux-musleabihf ;; \
    *) exit 1 ;; \
    esac && \
    rustup target add $RUST_TARGET && \
    cargo build --release --target $RUST_TARGET && \
    mv target/$RUST_TARGET/release/rust-mdns-repeater /app/rust-mdns-repeater

FROM scratch
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group
COPY --from=builder /app/rust-mdns-repeater /app/rust-mdns-repeater
WORKDIR /app
USER user:user
ENTRYPOINT [ "/app/rust-mdns-repeater" ]
CMD [ "--help" ]