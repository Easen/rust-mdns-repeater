use clap::Parser;
use env_logger::Env;
use ipnet::Ipv4Net;
use log::{debug, error, info};
use nix::libc::{c_char, ifreq, SIOCGIFADDR, SIOCGIFNETMASK};
use nix::sys::epoll::*;
use nix::sys::socket::sockopt::{
    BindToDevice, IpAddMembership, IpMulticastLoop, Ipv4PacketInfo, ReuseAddr,
};
use nix::sys::socket::*;
use std::error::Error;
use std::ffi::OsString;
use std::mem;
use std::net::Ipv4Addr;
use std::os::fd::RawFd;
use std::os::fd::{AsRawFd, OwnedFd};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MDNS_PORT: u16 = 5353;
const MDNS_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

nix::ioctl_read_bad!(siocgifaddr, SIOCGIFADDR, ifreq);
nix::ioctl_read_bad!(siocgifnetmask, SIOCGIFNETMASK, ifreq);

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Interfaces
    #[arg(short, long)]
    interfaces: Vec<String>,

    #[arg(short, long)]
    verbose: bool,
}

#[derive(Debug)]
struct Interface {
    pub name: String,
    pub network: Ipv4Net,
    sockfd: OwnedFd,
}

fn ifreq_for(name: &str) -> ifreq {
    let mut req: ifreq = unsafe { mem::zeroed() };
    for (i, byte) in name.as_bytes().iter().enumerate() {
        req.ifr_name[i] = *byte as c_char
    }
    req
}

fn sockaddr_to_ipv4addr(addr: sockaddr) -> Result<Ipv4Addr> {
    Ok(Ipv4Addr::new(
        addr.sa_data[2].try_into()?,
        addr.sa_data[3].try_into()?,
        addr.sa_data[4].try_into()?,
        addr.sa_data[5].try_into()?,
    ))
}

impl Interface {
    fn new(interface_name: &String) -> Result<Self> {
        let sock_fd = socket(
            AddressFamily::Inet,
            SockType::Datagram,
            SockFlag::empty(),
            SockProtocol::Udp,
        )?;

        setsockopt(&sock_fd, BindToDevice, &OsString::from(&interface_name))?;

        let mut req = ifreq_for(interface_name);
        let addr: Ipv4Addr;
        unsafe {
            // get the ipv4 address
            siocgifaddr(sock_fd.as_raw_fd(), &mut req)?;
            addr = sockaddr_to_ipv4addr(req.ifr_ifru.ifru_addr)?;
        };

        let mask: Ipv4Addr;
        unsafe {
            // get the ipv4 mask
            siocgifnetmask(sock_fd.as_raw_fd(), &mut req)?;
            mask = sockaddr_to_ipv4addr(req.ifr_ifru.ifru_addr)?;
        };

        // add interface to the multicast-group
        setsockopt(
            &sock_fd,
            IpAddMembership,
            &IpMembershipRequest::new(MDNS_ADDR, Some(addr)),
        )?;

        Ok(Interface {
            name: interface_name.clone(),
            network: Ipv4Net::with_netmask(addr, mask)?,
            sockfd: sock_fd,
        })
    }

    fn network_contains_addr(&self, other: Ipv4Addr) -> bool {
        self.network.contains(&other)
    }
}

fn create_receiving_socket() -> Result<OwnedFd> {
    // create a UDP socket
    let recv_fd = socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::empty(),
        SockProtocol::Udp,
    )?;

    // reuse the address
    setsockopt(&recv_fd, ReuseAddr, &true)?;

    // bind the 0.0.0.0:5353
    let addr = SockaddrIn::new(0, 0, 0, 0, MDNS_PORT);
    bind(recv_fd.as_raw_fd(), &addr)?;

    // enable loopback, just in case someone else needs to the data
    setsockopt(&recv_fd, IpMulticastLoop, &true)?;

    setsockopt(&recv_fd, Ipv4PacketInfo, &true)?;

    Ok(recv_fd)
}

fn main() -> Result<()> {
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);

    let args = Args::parse();

    if args.interfaces.len() < 2 {
        panic!("At least 2 interfaces are required");
    }

    info!("Setting up the interfaces");
    let interfaces = args
        .interfaces
        .iter()
        .map(|interface_name| match Interface::new(interface_name) {
            Ok(interface) => {
                info!(
                    "interface {:?}: network {:?}",
                    interface_name, interface.network
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

    info!("Creating receiving socket");
    let recv_fd = match create_receiving_socket() {
        Ok(recv_fd) => recv_fd,
        Err(err) => {
            error!("Unable to create receiving socket - {:?}", err);
            return Err(err);
        }
    };

    info!("Setting up epoll");
    let epoll = Epoll::new(EpollCreateFlags::empty())?;
    let mut epoll_events = vec![EpollEvent::empty(); 16];
    let event = EpollEvent::new(EpollFlags::EPOLLIN, recv_fd.as_raw_fd() as u64);
    epoll.add(&recv_fd, event)?;

    info!("Starting poll...");
    let dst: SockaddrIn = SockaddrIn::new(224, 0, 0, 251, MDNS_PORT);
    loop {
        let num = epoll.wait(&mut epoll_events, 100)?;

        'events: for i in 0..num {
            let mut buf: [u8; 4096] = [0; 4096];
            let sockfd = epoll_events[i].data() as RawFd;
            let (len, addr) = recvfrom::<SockaddrIn>(sockfd, &mut buf)?;

            if addr.is_none() {
                continue 'events;
            }
            let addr = Ipv4Addr::from(addr.unwrap().ip());

            // check for loopback
            let loop_back_interface = &interfaces.iter().find(|x| x.network.addr() == addr);
            if loop_back_interface.is_some() {
                debug!("Ignoring loopback a MDNS packet from {:?}", addr);
                continue 'events;
            }

            let src_interface = interfaces
                .iter()
                .find(|interface| interface.network_contains_addr(addr));

            if src_interface.is_none() {
                debug!(
                    "Ignoring a MDNS packet from an unknown interface from {:?}",
                    addr
                );
                continue 'events;
            }
            let src_interface = src_interface.unwrap();

            debug!(
                "Received MDNS packets from {:?} from {:?} (sockfd: {})",
                addr, src_interface.name, sockfd
            );

            interfaces
                .iter()
                .filter(|interface| !interface.name.eq(&src_interface.name))
                .for_each(|interface| {
                    match sendto(interface.sockfd.as_raw_fd(), &buf[0..len], &dst, MsgFlags::empty()) {
                        Err(err) => {
                            error!("Unable to forward MDNS packets from {:?} to {:?} due to error - {:?}",  addr, interface.name, err)
                        }
                        Ok(_) => info!(
                            "Forwared MDNS packets from {:?} to {:?}",
                            addr, interface.name
                        ),
                    }
                });
        }
    }
}
