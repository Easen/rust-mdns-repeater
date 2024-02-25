use clap::Parser;
use dns_parser::Packet;
use env_logger::Env;
use ipnet::Ipv4Net;
use log::Level::Trace;
use log::{debug, error, info, log_enabled, trace};
use nix::sys::epoll::*;
use nix::sys::socket::*;
use std::collections::HashMap;
use std::error::Error;
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::os::fd::RawFd;

mod interface;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Interfaces
    #[arg(short, long)]
    interface: Vec<String>,

    /// Additional subnets that will be repeated to the other interfaces
    #[arg(long)]
    additional_subnet: Vec<String>,

    /// Ignore mDNS question/queries from these interfaces
    #[arg(long)]
    ignore_question_subnet: Vec<String>,
}

fn main() -> Result<()> {
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);

    let args = Args::parse();

    if args.interface.len() < 2 {
        panic!("At least 2 interfaces are required");
    }

    let aditional_subnets = args
        .additional_subnet
        .iter()
        .map(|s| {
            let subnet = s.parse().unwrap();
            info!("allowed_subenet = {:?}", subnet);
            subnet
        })
        .collect::<Vec<Ipv4Net>>();

    let ignore_question_subnets = args
        .ignore_question_subnet
        .iter()
        .map(|s| {
            let subnet = s.parse().unwrap();
            info!("ignore_question_subnet = {:?}", subnet);
            subnet
        })
        .collect::<Vec<Ipv4Net>>();

    debug!("Setting up the interfaces");
    let interfaces = args
        .interface
        .iter()
        .map(
            |interface_name| match interface::Interface::new(interface_name) {
                Ok(interface) => {
                    info!(
                        "interface {:?}: ipv4 {:?}",
                        interface_name,
                        interface.ipv4_addr()
                    );
                    return interface;
                }
                Err(err) => panic!(
                    "Error occurred while establishing interface {:?} - {:?}",
                    interface_name,
                    err.to_string()
                ),
            },
        )
        .collect::<Vec<interface::Interface>>();

    debug!("Setting up the epoll");
    let epoll = Epoll::new(EpollCreateFlags::empty()).unwrap();
    let mut epoll_events = vec![EpollEvent::empty(); 16];

    info!("Setting up server sockets");
    let mut rx_socks = HashMap::new();
    interfaces.iter().for_each(|interface| {
        let fd = interface.rx_fd();
        let event = EpollEvent::new(EpollFlags::EPOLLIN, fd.as_raw_fd() as u64);
        epoll.add(&fd, event).unwrap();
        rx_socks.insert(fd.as_raw_fd(), interface);
    });

    let dst: SockaddrIn = SockaddrIn::new(224, 0, 0, 251, interface::MDNS_PORT);
    loop {
        let num = epoll.wait(&mut epoll_events, 1000).unwrap();
        trace!("Received {} events", num);
        'events: for i in 0..num {
            let mut buf: [u8; 4096] = [0; 4096];
            let sockfd = epoll_events[i].data() as RawFd;
            let (len, addr) = recvfrom::<SockaddrIn>(sockfd, &mut buf).unwrap();
            if addr.is_none() {
                continue;
            }
            let addr = Ipv4Addr::from(addr.unwrap().ip());
            let data = &buf[0..len];
            let src_interface = rx_socks.get(&sockfd);
            if src_interface.is_none() {
                debug!(
                    "Ignoring a mDNS packet from an unknown interface from {:?}",
                    addr
                );
                continue 'events;
            }
            let src_interface = src_interface.unwrap();

            // ignore loopbacks
            if src_interface.ipv4_addr() == addr {
                trace!(
                    "Ignoring loopback mDNS packet from {:?} - {:?}",
                    src_interface.name(),
                    addr
                );
                continue 'events;
            }

            if log_enabled!(Trace) {
                let dns_packet = Packet::parse(data);
                if dns_packet.is_ok() {
                    let dns_packet = dns_packet.unwrap();
                    trace!(
                        "Parsed mDNS packet from {:?} from {:?}- {:?}",
                        addr,
                        src_interface.name(),
                        dns_packet
                    );
                }
            } else {
                debug!(
                    "Received mDNS packets from {:?} from {:?})",
                    addr,
                    src_interface.name()
                );
            }

            if !src_interface.network_contains_addr(addr) {
                let allowed_subnet = aditional_subnets.iter().find(|i| i.contains(&addr));
                if allowed_subnet.is_none() {
                    info!(
                        "Ignoring mDNS packet from {:?} that originates from outside the source network {:?}",
                        addr,
                        src_interface.ipv4_addr()
                    );
                    continue 'events;
                }
                debug!(
                    "Allowing mDNS packet from {:?} that originates from outside the source network {:?} (allowed subnet {:?}",
                    addr,
                    src_interface.name(),
                    allowed_subnet.unwrap()
                );
            }

            if ignore_question_subnets.len() > 0 {
                if let Some(ignored_subnet) =
                    ignore_question_subnets.iter().find(|x| x.contains(&addr))
                {
                    let dns_packet = Packet::parse(data);
                    if dns_packet.is_ok() && dns_packet?.questions.len() > 0 {
                        info!(
                            "Ignoring mDNS question from {:?} as it originates from the subnet {:?} (interface {:?})",
                            addr,
                            ignored_subnet,
                            src_interface.name()
                        );
                        continue 'events;
                    }
                }
            }

            interfaces
                .iter()
                .filter(|interface| !interface.name().eq(src_interface.name()))
                .for_each(|interface| {
                    match sendto(interface.tx_fd().as_raw_fd(), data, &dst, MsgFlags::empty()) {
                        Err(err) => {
                            error!("Unable to forward mDNS packets from {:?} to {:?} due to error - {:?}",  addr, interface.name(), err)
                        }
                        Ok(_) => info!(
                            "Forwarded mDNS packets ({} bytes) from {:?} to {:?} ",
                            data.len(), addr, interface.name()
                        ),
                    }
                });
        }
    }
}
