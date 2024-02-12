#![allow(dead_code)]
#![allow(unused_assignments)]
#![allow(unused_variables)]
#![allow(unused_imports)]
use std::error::Error;
use std::mem;
use std::net::Ipv4Addr;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};

use clap::Parser;
use log::{debug, error, info};
use std::ffi::OsString;
// use nix::sys::epoll::*;
use env_logger::Env;
use ipnet::Ipv4Net;
use nix::ioctl_read;
use nix::libc::{c_char, ifreq, in_addr, SIOCGIFADDR, SIOCGIFNETMASK};
use nix::sys::socket::sockopt::{
    BindToDevice, IpAddMembership, IpMulticastLoop, Ipv4PacketInfo, ReuseAddr,
};
use nix::sys::socket::*;
use nix::*;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MDNS_PORT: u16 = 5353;
const MDNS_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

nix::ioctl_read_bad!(siocgifaddr, SIOCGIFADDR, ifreq);
nix::ioctl_read_bad!(siocgifnetmask, SIOCGIFNETMASK, ifreq);

/// Simple program to greet a person
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
    name: String,
    addr: Ipv4Addr,
    mask: Ipv4Addr,
    network: Ipv4Net,
    sockfd: OwnedFd,
}

fn ifreq_for(name: &str) -> ifreq {
    let mut req: ifreq = unsafe { mem::zeroed() };
    for (i, byte) in name.as_bytes().iter().enumerate() {
        req.ifr_name[i] = *byte as c_char
    }
    req
}

fn sockaddr_to_ipv4addr(addr: sockaddr) -> Ipv4Addr {

    std::net::IpAddr::new()
    return Ipv4Addr::new(
        addr.sa_data[2],
        addr.sa_data[3],
        addr.sa_data[4],
        addr.sa_data[5],
    );
}

impl Interface {
    fn new(interface: &String) -> Result<Self> {
        let sockfd = socket(
            AddressFamily::Inet,
            SockType::Datagram,
            SockFlag::empty(),
            SockProtocol::Udp,
        )?;

        setsockopt(&sockfd, BindToDevice, &OsString::from(&interface))?;

        let mut req = ifreq_for(interface);
        let addr: Ipv4Addr;
        unsafe {
            siocgifaddr(sockfd.as_raw_fd(), &mut req)?;
            addr = sockaddr_to_ipv4addr(req.ifr_ifru.ifru_addr);
        };

        let mask: Ipv4Addr;
        unsafe {
            siocgifnetmask(sockfd.as_raw_fd(), &mut req)?;
            mask = sockaddr_to_ipv4addr(req.ifr_ifru.ifru_addr);
        };

        Ok(Interface {
            name: interface.clone(),
            addr,
            mask,
            network: Ipv4Net::with_netmask(addr, mask)?,
            sockfd,
        })
    }

    // fn has(&self, addr: std::net::IpAddr) -> bool {
    //     return self.address == addr;
    // }
}

fn main() -> Result<()> {
    let env = Env::default().filter_or("LOG", "info");
    env_logger::init_from_env(env);

    let args = Args::parse();

    if args.interfaces.len() < 2 {
        panic!("At least 2 interfaces are required");
    }

    info!("Setting up the interfaces");
    let interfaces: Vec<_> = args
        .interfaces
        .iter()
        .map(|inter| Interface::new(inter).unwrap())
        .collect();

    info!("Creating receiving socket");
    let recv_fd = create_receiving_socket(interfaces)?;

    info!("Setting up epoll");

    Ok(())
}

fn create_receiving_socket(interfaces: Vec<Interface>) -> Result<OwnedFd> {
    // create a UDP socket
    let recv_fd = socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::empty(),
        SockProtocol::Udp,
    )?;

    // bind the 0.0.0.0:5353
    let addr = SockaddrIn::new(0, 0, 0, 0, MDNS_PORT);
    bind(recv_fd.as_raw_fd(), &addr)?;

    // enable loopback, just in case someone else needs to the data
    setsockopt(&recv_fd, IpMulticastLoop, &true)?;

    for interface in interfaces {
        let membership_request = IpMembershipRequest::new(MDNS_ADDR, Some(interface.addr));
        setsockopt(&recv_fd, IpAddMembership, &membership_request)?;
    }

    Ok(recv_fd)
}
