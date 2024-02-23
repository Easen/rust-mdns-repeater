use ipnet::Ipv4Net;
use nix::libc::{c_char, ifreq, SIOCGIFADDR, SIOCGIFNETMASK};
use nix::sys::socket::sockopt::{
    BindToDevice, IpAddMembership, IpMulticastLoop, Ipv4PacketInfo, Ipv4Ttl, ReuseAddr,
};
use nix::sys::socket::*;
use std::error::Error;
use std::ffi::OsString;
use std::mem;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, OwnedFd};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub const MDNS_PORT: u16 = 5353;
pub const MDNS_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

nix::ioctl_read_bad!(siocgifaddr, SIOCGIFADDR, ifreq);
nix::ioctl_read_bad!(siocgifnetmask, SIOCGIFNETMASK, ifreq);

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

fn get_network_for_interface(interface_name: &String, sock_fd: &OwnedFd) -> Result<Ipv4Net> {
    let mut req = ifreq_for(&interface_name);
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

    Ok(Ipv4Net::with_netmask(addr, mask)?)
}
fn create_udp_multicast_sock(interface_name: &String) -> Result<OwnedFd> {
    let sock = socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::empty(),
        SockProtocol::Udp,
    )?;

    setsockopt(&sock, BindToDevice, &OsString::from(&interface_name))?;
    setsockopt(&sock, ReuseAddr, &true)?;
    setsockopt(&sock, IpMulticastLoop, &true)?;
    setsockopt(&sock, Ipv4PacketInfo, &true)?;

    Ok(sock)
}

#[derive(Debug)]
pub struct Interface {
    pub name: String,
    pub network: Ipv4Net,
    pub tx_sock: OwnedFd,
    pub rx_sock: OwnedFd,
}

impl Interface {
    pub fn new(interface_name: &String) -> Result<Self> {
        let tx_sock = create_udp_multicast_sock(interface_name)?;
        let network = get_network_for_interface(interface_name, &tx_sock)?;
        let sock_addr = &SockaddrIn::from(SocketAddrV4::new(network.addr(), MDNS_PORT));
        bind(tx_sock.as_raw_fd(), sock_addr)?;
        setsockopt(
            &tx_sock,
            IpAddMembership,
            &IpMembershipRequest::new(MDNS_ADDR, Some(network.addr())),
        )?;
        setsockopt(&tx_sock, Ipv4Ttl, &255)?;

        let rx_sock = create_udp_multicast_sock(interface_name)?;
        let sock_addr = &SockaddrIn::from(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), MDNS_PORT));
        bind(rx_sock.as_raw_fd(), sock_addr)?;

        Ok(Interface {
            name: interface_name.clone(),
            network,
            tx_sock,
            rx_sock,
        })
    }

    pub fn network_contains_addr(&self, other: Ipv4Addr) -> bool {
        self.network.contains(&other)
    }
}
