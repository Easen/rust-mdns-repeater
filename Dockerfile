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

RUN apt update && apt install -y musl-tools musl-dev
RUN update-ca-certificates

# Create appuser
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
RUN cargo build --target $(cat /$TARGETARCH.txt) --release --bins
RUN mv /app/target/$(cat /$TARGETARCH.txt)/release/rust-mdns-repeater /app/rust-mdns-repeater

FROM scratch
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group
WORKDIR /app
COPY --from=builder /app/rust-mdns-repeater /app/rust-mdns-repeater
USER user:user
ENTRYPOINT [ "/app/rust-mdns-repeater" ]
CMD [ "--help" ]