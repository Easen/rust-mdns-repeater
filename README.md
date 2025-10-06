# rust-mdns-repeater

Small daemon that forwards mDNS (UDP/5353) between network interfaces. Useful when you need mDNS discovery to cross network segments (for example between isolated VLANs) and you understand the security implications of repeating multicast traffic.

This utility is written in Rust and uses low-level sockets; it is intended to run on Linux systems.

## Features

- Forward IPv4 and IPv6 mDNS traffic between multiple interfaces
- Allow additional IPv4 subnets to be treated as local (repeat into the network)
- Optionally ignore mDNS questions originating from configured subnets
- Configurable logging and error handling

## Requirements

- Linux (binary built for Linux)
- Rust toolchain for building: `rustup`, `cargo`
- Permissions to create multicast sockets (usually requires root or CAP_NET_RAW)

## Building

### Native (on Linux)

```bash
cargo build --release
```

Binary will be available at `target/release/rust-mdns-repeater`.

## Running

Minimum: two interfaces must be provided.

```bash
sudo ./rust-mdns-repeater --interface eth0 --interface eth1
```

Common flags

- `--interface <NAME>` (repeatable) — interface names to listen on (required, at least 2)
- `--additional-subnet <CIDR>` (repeatable) — IPv4 subnet(s) allowed to be treated as local
- `--ignore-question-subnet <CIDR>` (repeatable) — subnets to ignore mDNS questions from
- `--error-instead-of-exit` — log forwarding errors instead of panicking/exiting
- `--disable-ipv4` / `--disable-ipv6` — disable IPv4/IPv6 listeners

Examples

Forward between `eth0` and `eth1`, allow `10.0.0.0/8` as local:

```bash
sudo ./rust-mdns-repeater \
  --interface eth0 --interface eth1 \
  --additional-subnet 10.0.0.0/8
```

Ignore mDNS questions from a management network:

```bash
sudo ./rust-mdns-repeater --interface br0 --interface eth0 \
  --ignore-question-subnet 192.168.100.0/24
```

### full example using docker compose


docker-compose.yml:

```yaml
version: "3"
services:
  mdns-repeater:
    image: ghcr.io/easen/rust-mdns-repeater:latest
    command: "--interface end0 --interface end0.20 --interface end0.30 --additional-subnet 192.168.10.0/24 --ignore-question-subnet 10.1.30.0/24"
    environment:
      RUST_LOG: "WARN"
    network_mode: host
    restart: unless-stopped
```

This will forward mDNS packets between end0 (vlan 10 - management devices, routers, switches, HA, etc.), end0.20 (vlan 20 - pc, mobiles) & end0.30 (vlan 30 - IOT devices), including packets from 192.168.10.0/24 (a docker mac-vlan from a NAS hosted in vlan 10) but will ignore mDNS questions from 10.1.30.0/24 (IOT device range). 

This means IOT devices are discoverable on vlan 10, 20, but devices on 10.1.30.0/24 (vlan 30) cannot query devices on the other vlans.

## Logging

Logging is controlled with `RUST_LOG`. Example:

```bash
RUST_LOG=info sudo ./rust-mdns-repeater --interface eth0 --interface eth1
RUST_LOG=trace sudo ./rust-mdns-repeater ...   # verbose
```

## Troubleshooting

- Permissions: running requires root or capabilities to open multicast sockets (CAP_NET_RAW). Use `sudo` or grant capabilities.
- Platform: the program is intended to run on Linux. Cross-compiled Linux binaries will not run on macOS, windows, etc.


## Contributing

Feel free to open issues and pull requests. 

## License

See repository LICENSE.
