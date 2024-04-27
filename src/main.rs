use clap::Parser;
use dns_parser::Packet;
use env_logger::Env;
use ipnet::IpNet;
use log::Level::Trace;
use log::{debug, error, info, log_enabled, trace};
use nix::sys::epoll::*;
use nix::sys::socket::*;
use std::collections::HashMap;
use std::error::Error;
use std::net::{IpAddr, SocketAddrV4, SocketAddrV6};
use std::os::fd::AsRawFd;
use std::os::fd::RawFd;
use mimalloc::MiMalloc;

mod interface;
use interface::{Interface, InterfaceV4, InterfaceV6, IPV4_MDNS_ADDR, IPV6_MDNS_ADDR};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use crate::interface::MDNS_PORT;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const EPOLL_TIMEOUT: u16 = 100;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Interfaces
    #[arg(short, long)]
    interface: Vec<String>,

    /// Additional subnets that will be repeated to the other interfaces (Ipv4 only)
    #[arg(long)]
    additional_subnet: Vec<String>,

    /// Ignore mDNS question/queries from these IPv4/IPv6 Subnets
    #[arg(long)]
    ignore_question_subnet: Vec<String>,
    
    /// Log errors instead of exiting when an errors occurs during forwarding
    #[arg(long)]
    error_instead_of_exit: bool,

    #[arg(long, default_value_t = false)]
    disable_ipv4: bool,

    #[arg(long, default_value_t = false)]
    disable_ipv6: bool,
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
        .collect::<Vec<IpNet>>();

    let ignore_question_subnets = args
        .ignore_question_subnet
        .iter()
        .map(|s| {
            let subnet = s.parse().unwrap();
            info!("ignore_question_subnet = {:?}", subnet);
            subnet
        })
        .collect::<Vec<IpNet>>();

    debug!("Setting up the interfaces");
    let mut interfaces = Vec::new();

    if !args.disable_ipv4 {
        let mut ipv4_interfaces: Vec<Interface> = args
            .interface
            .iter()
            .map(|interface_name| match InterfaceV4::new(interface_name) {
                Ok(interface) => {
                    let interface = Interface::V4(interface);
                    info!(
                        "interface {:?}: ipv4 {:?}",
                        interface_name,
                        interface.addr()
                    );
                    return interface;
                }
                Err(err) => panic!(
                    "Error occurred while establishing interface {:?} - {:?}",
                    interface_name,
                    err.to_string()
                ),
            })
            .collect::<Vec<Interface>>();

        interfaces.append(&mut ipv4_interfaces);
    };

    if !args.disable_ipv6 {
        let mut ipv6_interfaces: Vec<Interface> = args
            .interface
            .iter()
            .map(|interface_name| match InterfaceV6::new(interface_name) {
                Ok(interface) => {
                    let interface = Interface::V6(interface);
                    info!(
                        "interface {:?}: ipv6 {:?}",
                        interface_name,
                        interface.addr()
                    );
                    return interface;
                }
                Err(err) => panic!(
                    "Error occurred while establishing interface {:?} - {:?}",
                    interface_name,
                    err.to_string()
                ),
            })
            .collect::<Vec<Interface>>();
        interfaces.append(&mut ipv6_interfaces);
    }

    debug!("Setting up the epoll");
    let epoll = Epoll::new(EpollCreateFlags::empty()).unwrap();
    let mut epoll_events = vec![EpollEvent::empty(); 16];

    info!("Setting up server sockets");
    let mut rx_socks = HashMap::new();
    interfaces.iter().for_each(|interface| {
        let rx_fd = interface.rx_fd();
        let event = EpollEvent::new(EpollFlags::EPOLLIN, rx_fd.as_raw_fd() as u64);
        epoll.add(&rx_fd, event).unwrap();
        rx_socks.insert(rx_fd.as_raw_fd(), interface);
    });

    let ipv4_dst: SockaddrIn = SockaddrIn::from(SocketAddrV4::new(IPV4_MDNS_ADDR, MDNS_PORT));
    let ipv6_dst: SockaddrIn6 =
        SockaddrIn6::from(SocketAddrV6::new(IPV6_MDNS_ADDR, MDNS_PORT, 0, 0));

    loop {
        let result = epoll.wait(&mut epoll_events, EPOLL_TIMEOUT);
        if result.is_err() {
            error!("Error from epoll {:?}", result.err());
            continue;
        }
        let num = result.unwrap();
        if num == 0 {
            continue;
        }
        trace!("Received {} events", num);
        'events: for i in 0..num {
            let mut buf: [u8; 4096] = [0; 4096];
            let sockfd = epoll_events[i].data() as RawFd;
            // find the interface for the sockfd
            let src_interface = rx_socks.get(&sockfd);

            if src_interface.is_none() {
                debug!("Ignoring a mDNS packet from an unknown interface");
                continue 'events;
            }

            let (len, addr) = match src_interface.unwrap() {
                Interface::V4(_interface) => {
                    let (len, addr) = recvfrom::<SockaddrIn>(sockfd, &mut buf).unwrap();
                    if addr.is_none() {
                        continue;
                    }
                    (len, IpAddr::V4(addr.unwrap().ip()))
                }
                Interface::V6(_interface) => {
                    let (len, addr) = recvfrom::<SockaddrIn6>(sockfd, &mut buf).unwrap();
                    if addr.is_none() {
                        continue;
                    }
                    (len, IpAddr::V6(addr.unwrap().ip()))
                }
            };

            // let addr = Ipv4Addr::from(addr.unwrap().ip());
            let data = &buf[0..len];

            let src_interface = src_interface.unwrap();

            // ignore loopbacks
            if src_interface.addr() == addr {
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
                    "Received  mDNS packets from {:?} from {:?}) ({} bytes)",
                    addr,
                    src_interface.name(),
                    data.len()
                );
            }

            match src_interface {
                Interface::V4(_) => {
                    if !src_interface.network_contains_addr(addr) {
                        let allowed_subnet = aditional_subnets.iter().find(|i| i.contains(&addr));
                        if allowed_subnet.is_none() {
                            info!(
                                "Ignoring mDNS packet from {:?} that originates from outside the source network {:?}",
                                addr,
                                src_interface.addr()
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
                }
                _ => {}
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

            let is_src_interface_ipv4 = match src_interface {
                Interface::V4(_) => true,
                _ => false,
            };

            interfaces
                .iter()
                .filter(|interface| !interface.name().eq(src_interface.name()))
                .filter(|interface| match interface {
                    Interface::V4(_) => is_src_interface_ipv4,
                    Interface::V6(_) => !is_src_interface_ipv4,

                })
                .for_each(|interface| {
                    let dst: &dyn SockaddrLike = match interface {
                        Interface::V4(_) => &ipv4_dst,
                        Interface::V6(_) => &ipv6_dst,
    
                    };
                    match sendto(interface.tx_fd().as_raw_fd(), data, dst, MsgFlags::empty()) {
                        Err(err) => {
                            let error_message = format!("Unable to forward mDNS packets from {:?} to {:?} due to error - {:?}", addr, interface.name(), err);
                            if !args.error_instead_of_exit {
                                panic!("{}", error_message);
                            }
                            error!("{}", error_message);
                        }
                        Ok(_) => info!(
                            "Forwarded mDNS packets from {:?} to {:?} ({} bytes)",
                             addr, interface.name(),data.len()
                        ),
                    }
                });
        }
    }
}
