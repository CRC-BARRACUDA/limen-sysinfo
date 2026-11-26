//! Network adapters, addresses and where traffic leaves by.

use std::net::{Ipv4Addr, Ipv6Addr};

use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::Networking::WinSock::SOCKET_ADDRESS;

use super::{from_pwstr, Facts};

const ERROR_BUFFER_OVERFLOW: u32 = 111;
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const OPER_STATUS_UP: i32 = 1;
const OPER_STATUS_DOWN: i32 = 2;

pub(super) fn collect(f: &mut Facts) {
    let Some(buf) = adapters() else {
        return;
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut route: Option<String> = None;
    let mut dns: Vec<String> = Vec::new();

    // SAFETY: `buf` is the list as the system wrote it, and outlives the walk.
    // Every pointer followed is one the system put in that list, and each is
    // checked for null before it is.
    unsafe {
        let mut at = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !at.is_null() {
            let a = &*at;
            at = a.Next;
            // The loopback is not an adapter, and what has no hardware address
            // is a tunnel Windows keeps for itself, not a way out of the
            // machine.
            if a.IfType == IF_TYPE_SOFTWARE_LOOPBACK || a.PhysicalAddressLength == 0 {
                continue;
            }
            let name = from_pwstr(a.FriendlyName);
            let up = a.OperStatus == OPER_STATUS_UP;

            let mut parts: Vec<String> = Vec::new();
            let mut unicast = a.FirstUnicastAddress;
            while !unicast.is_null() {
                let u = &*unicast;
                unicast = u.Next;
                // The IPv4 address, as on Linux: it is the one written on the
                // ticket, and an adapter's IPv6 addresses are a list of their
                // own.
                if let Some(Address::V4(ip)) = address(&u.Address) {
                    parts.push(format!("{ip}/{}", u.OnLinkPrefixLength));
                }
            }
            let mac = &a.PhysicalAddress[..(a.PhysicalAddressLength as usize).min(8)];
            parts.push(format!(
                "MAC {}",
                mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
            ));
            match a.OperStatus {
                OPER_STATUS_UP => parts.push("up".into()),
                OPER_STATUS_DOWN => parts.push("down".into()),
                _ => {}
            }
            // A link that is down reports the speed it does not have as all
            // ones.
            if up && a.TransmitLinkSpeed > 0 && a.TransmitLinkSpeed != u64::MAX {
                parts.push(format!("{} Mb/s", a.TransmitLinkSpeed / 1_000_000));
            }
            if a.Mtu > 0 && a.Mtu != u32::MAX {
                parts.push(format!("MTU {}", a.Mtu));
            }
            // What the adapter *is* — "Ethernet 2" is a name somebody gave it.
            let description = from_pwstr(a.Description);
            if !description.is_empty() {
                parts.push(description);
            }
            rows.push((name.clone(), parts.join(", ")));

            if !up {
                continue;
            }
            let mut gateway = a.FirstGatewayAddress;
            while !gateway.is_null() {
                let g = &*gateway;
                gateway = g.Next;
                if let (None, Some(Address::V4(gw))) = (&route, address(&g.Address)) {
                    route = Some(format!("via {gw} on {name}"));
                }
            }
            let mut server = a.FirstDnsServerAddress;
            while !server.is_null() {
                let s = &*server;
                server = s.Next;
                let text = match address(&s.Address) {
                    Some(Address::V4(ip)) => ip.to_string(),
                    // `fec0:0:0:ffff::1` and its two neighbours are not
                    // servers anybody set: Windows lists them on any adapter
                    // that was given none.
                    Some(Address::V6(ip)) if ip.segments()[0] != 0xfec0 => ip.to_string(),
                    _ => continue,
                };
                if !dns.contains(&text) {
                    dns.push(text);
                }
            }
        }
    }

    rows.sort();
    for (name, value) in rows {
        f.say("network", &name, value);
    }
    f.say("network", "Default route", route.unwrap_or_default());
    f.say("network", "DNS", dns.join(", "));
}

/// The system's list of adapters, in the buffer it was written to.
///
/// The size has to be guessed and the call says when the guess was short —
/// more than once, if an adapter appears between asking and being told.
fn adapters() -> Option<Vec<u64>> {
    const AF_UNSPEC: u32 = 0;
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size: u32 = 16 * 1024;
    for _ in 0..4 {
        // Of `u64`, so that the structures the system writes are aligned.
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: `size` is the size of `buf` in bytes, or less.
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC,
                flags,
                std::ptr::null(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match result {
            0 => return Some(buf),
            ERROR_BUFFER_OVERFLOW => continue, // `size` now says how much
            _ => return None,
        }
    }
    None
}

enum Address {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

/// The address in a socket address, whichever family it is.
///
/// # Safety
/// `socket` is one the system filled in: its pointer is null or valid for the
/// length beside it.
unsafe fn address(socket: &SOCKET_ADDRESS) -> Option<Address> {
    const AF_INET: u16 = 2;
    const AF_INET6: u16 = 23;
    if socket.lpSockaddr.is_null() || socket.iSockaddrLength < 2 {
        return None;
    }
    let raw = std::slice::from_raw_parts(
        socket.lpSockaddr.cast::<u8>(),
        socket.iSockaddrLength as usize,
    );
    // Family, port, then the address: at 4 for IPv4, and for IPv6 at 8, after
    // the flow label.
    match u16::from_ne_bytes([raw[0], raw[1]]) {
        AF_INET => {
            let b: [u8; 4] = raw.get(4..8)?.try_into().ok()?;
            Some(Address::V4(Ipv4Addr::from(b)))
        }
        AF_INET6 => {
            let b: [u8; 16] = raw.get(8..24)?.try_into().ok()?;
            Some(Address::V6(Ipv6Addr::from(b)))
        }
        _ => None,
    }
}
