use ipnet::{Ipv4Net, Ipv6Net};
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::libc::{c_char, if_nametoindex, ifreq, O_NONBLOCK, SIOCGIFADDR, SIOCGIFNETMASK};
use nix::sys::socket::sockopt::{
    BindToDevice, IpAddMembership, IpMulticastLoop, Ipv4PacketInfo, Ipv4Ttl, Ipv6RecvPacketInfo,
    Ipv6V6Only, ReuseAddr,
};
use nix::sys::socket::*;
use std::error::Error;
use std::ffi::OsString;
use std::io;
use std::mem::{self};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
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
    let mut req: ifreq = ifreq_for(&interface_name);
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

enum SockDirection {
    RX,
    TX,
}

const ON: bool = true;

fn create_udp_sock(
    interface_name: &String,
    domain: AddressFamily,
    sock_direction: SockDirection,
) -> Result<OwnedFd> {
    let sock = socket(
        domain,
        SockType::Datagram,
        SockFlag::empty(),
        SockProtocol::Udp,
    )?;

    match domain {
        AddressFamily::Inet => {
            setsockopt(&sock, IpMulticastLoop, &ON)?;
            match sock_direction {
                SockDirection::RX => {
                    setsockopt(&sock, Ipv4PacketInfo, &ON)?;
                    setsockopt(&sock, BindToDevice, &OsString::from(&interface_name))?;
                }
                SockDirection::TX => unsafe {
                    let ifreq = ifreq_for(&interface_name);
                    let ifindex = ifreq.ifr_ifru.ifru_ifindex;
                    let res = nix::libc::setsockopt(
                        sock.as_raw_fd(),
                        nix::libc::IPPROTO_IP,
                        nix::libc::IP_MULTICAST_IF,
                        &ifindex as *const _ as *const nix::libc::c_void,
                        mem::size_of_val(&ifindex) as nix::libc::socklen_t,
                    );
                    if res != 0 {
                        return Err(Box::new(io::Error::last_os_error()));
                    }
                },
            }
        }
        AddressFamily::Inet6 => {
            setsockopt(&sock, Ipv6V6Only, &ON)?;
            match sock_direction {
                SockDirection::RX => {
                    setsockopt(&sock, Ipv6RecvPacketInfo, &ON)?;
                    setsockopt(&sock, BindToDevice, &OsString::from(&interface_name))?;
                }
                SockDirection::TX => unsafe {
                    let ifindex =
                        if_nametoindex(&interface_name as *const _ as *const nix::libc::c_char);
                    let res = nix::libc::setsockopt(
                        sock.as_raw_fd(),
                        nix::libc::IPPROTO_IPV6,
                        nix::libc::IPV6_MULTICAST_IF,
                        &ifindex as *const _ as *const nix::libc::c_void,
                        mem::size_of_val(&ifindex) as nix::libc::socklen_t,
                    );
                    if res != 0 {
                        return Err(Box::new(io::Error::last_os_error()));
                    }

                    // let res = nix::libc::setsockopt(
                    //     sock.as_raw_fd(),
                    //     nix::libc::IPPROTO_IPV6,
                    //     nix::libc::IPV6_MULTICAST_LOOP,
                    //     &OFF as *const _ as *const nix::libc::c_void,
                    //     mem::size_of_val(&OFF) as nix::libc::socklen_t,
                    // );
                    // if res != 0 {
                    //     return Err(Box::new(io::Error::last_os_error()));
                    // }
                },
            }
        }
        _ => {}
    };
    setsockopt(&sock, ReuseAddr, &ON)?;

    let flags = fcntl(sock.as_raw_fd(), FcntlArg::F_GETFD)?;
    fcntl(
        sock.as_raw_fd(),
        FcntlArg::F_SETFL(OFlag::from_bits_retain(flags | O_NONBLOCK)),
    )?;

    Ok(sock)
}

pub enum Interface {
    V4(InterfaceV4),
    V6(InterfaceV6),
}

#[derive(Debug)]
pub struct InterfaceV4 {
    name: String,
    network: Ipv4Net,
    tx_sock: OwnedFd,
    rx_sock: OwnedFd,
}

pub struct InterfaceV6 {
    name: String,
    network: Ipv6Net,
    tx_sock: OwnedFd,
    rx_sock: OwnedFd,
}

impl Interface {
    pub fn name(&self) -> &String {
        match self {
            Interface::V4(x) => &x.name,
            Interface::V6(x) => &x.name,
        }
    }

    pub fn addr(&self) -> IpAddr {
        match self {
            Interface::V4(x) => IpAddr::V4(x.network.addr()),
            Interface::V6(x) => IpAddr::V6(x.network.addr()),
        }
    }

    pub fn rx_fd(&self) -> &OwnedFd {
        match self {
            Interface::V4(x) => &x.rx_sock,
            Interface::V6(x) => &x.rx_sock,
        }
    }

    pub fn tx_fd(&self) -> &OwnedFd {
        match self {
            Interface::V4(x) => &x.tx_sock,
            Interface::V6(x) => &x.tx_sock,
        }
    }

    pub fn network_contains_addr(&self, other: IpAddr) -> bool {
        match self {
            Interface::V4(x) => match other {
                IpAddr::V4(ip) => x.network.contains(&ip),
                IpAddr::V6(_) => false,
            },
            Interface::V6(x) => match other {
                IpAddr::V4(_) => false,
                IpAddr::V6(ip) => x.network.contains(&ip),
            },
        }
    }
}

impl InterfaceV4 {
    pub fn new(interface_name: &String) -> Result<Self> {
        let tx_sock = create_udp_sock(interface_name, AddressFamily::Inet, SockDirection::TX)?;
        let network = get_network_for_interface(interface_name, &tx_sock)?;
        let sock_addr = &SockaddrIn::from(SocketAddrV4::new(network.addr(), MDNS_PORT));
        bind(tx_sock.as_raw_fd(), sock_addr)?;
        setsockopt(
            &tx_sock,
            IpAddMembership,
            &IpMembershipRequest::new(MDNS_ADDR, Some(network.addr())),
        )?;
        setsockopt(&tx_sock, Ipv4Ttl, &255)?;

        let rx_sock = create_udp_sock(interface_name, AddressFamily::Inet, SockDirection::RX)?;
        let sock_addr = &SockaddrIn::from(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), MDNS_PORT));
        bind(rx_sock.as_raw_fd(), sock_addr)?;

        Ok(InterfaceV4 {
            name: interface_name.clone(),
            network,
            tx_sock,
            rx_sock,
        })
    }
}

impl InterfaceV6 {
    pub fn new(interface_name: &String) -> Result<Self> {
        let tx_sock = create_udp_sock(interface_name, AddressFamily::Inet6, SockDirection::TX)?;
        let rx_sock = create_udp_sock(interface_name, AddressFamily::Inet6, SockDirection::RX)?;
        let sock_addr = &SockaddrIn6::from(SocketAddrV6::new(
            Ipv6Addr::new(
                0xff02, 0x0000, 0x0000, 0x0000, 0x000, 0x0000, 0x0000, 0x00fb,
            ),
            MDNS_PORT,
            0,
            0,
        ));
        bind(rx_sock.as_raw_fd(), sock_addr)?;

        let network = Ipv6Net::new(Ipv6Addr::new(0xfd, 0, 0, 0, 0, 0, 0, 0), 24)?;
        Ok(InterfaceV6 {
            name: interface_name.clone(),
            network,
            tx_sock,
            rx_sock,
        })
    }
}
