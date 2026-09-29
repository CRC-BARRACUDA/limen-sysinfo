//! Network interfaces, addresses and where traffic leaves by.

use super::{read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    let v4 = ipv4_by_interface();
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return;
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "lo" {
            continue;
        }
        let dir = entry.path();
        let mac = read_trim(dir.join("address")).unwrap_or_default();
        let state = read_trim(dir.join("operstate")).unwrap_or_default();
        let mtu = read_trim(dir.join("mtu")).unwrap_or_default();
        // Speed is meaningless — and returns -1 — on a link that is down or on
        // a virtual interface, so it is only reported when it is a number.
        let speed = read_trim(dir.join("speed"))
            .filter(|s| s.parse::<i64>().is_ok_and(|v| v > 0))
            .map(|s| format!("{s} Mb/s"))
            .unwrap_or_default();

        let mut parts: Vec<String> = Vec::new();
        if let Some(addr) = v4.iter().find(|(i, _)| *i == name).map(|(_, a)| a.clone()) {
            parts.push(addr);
        }
        if !mac.is_empty() {
            parts.push(format!("MAC {mac}"));
        }
        if !state.is_empty() {
            parts.push(state);
        }
        if !speed.is_empty() {
            parts.push(speed);
        }
        if !mtu.is_empty() {
            parts.push(format!("MTU {mtu}"));
        }
        rows.push((name, parts.join(", ")));
    }
    rows.sort();
    for (name, value) in rows {
        f.say("network", &name, value);
    }

    if let Some((iface, gw)) = default_route() {
        f.say("network", "Default route", format!("via {gw} on {iface}"));
    }
    let dns = nameservers();
    if !dns.is_empty() {
        f.say("network", "DNS", dns.join(", "));
    }
}

/// IPv4 addresses per interface, read from `/proc/net/route` and
/// `/proc/net/fib_trie`.
///
/// There is no per-interface IPv4 file under /proc — `ip addr` asks the kernel
/// over netlink. `fib_trie` is the routing table as text, and the local
/// addresses in it are the ones marked `LOCAL`; the interface each belongs to
/// comes from matching the address against the routes in `/proc/net/route`.
fn ipv4_by_interface() -> Vec<(String, String)> {
    let locals = local_ipv4s();
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(routes) = read_trim("/proc/net/route") else {
        return out;
    };
    for addr in locals {
        let Some(octets) = parse_ipv4(&addr) else { continue };
        let value = u32::from_be_bytes(octets);
        // The interface whose subnet contains this address.
        for line in routes.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 8 {
                continue;
            }
            let (Ok(dest), Ok(mask)) = (
                u32::from_str_radix(cols[1], 16).map(u32::swap_bytes),
                u32::from_str_radix(cols[7], 16).map(u32::swap_bytes),
            ) else {
                continue;
            };
            if mask != 0 && value & mask == dest & mask {
                out.push((cols[0].to_string(), addr.clone()));
                break;
            }
        }
    }
    out
}

/// Every address this machine answers to, out of `fib_trie`'s LOCAL entries.
fn local_ipv4s() -> Vec<String> {
    let Some(trie) = read_trim("/proc/net/fib_trie") else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = trie.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let Some(addr) = line.trim().strip_prefix("|-- ") else { continue };
        // The kind of address is on the following line: `/32 host LOCAL`.
        if lines.get(i + 1).is_some_and(|n| n.contains("LOCAL")) {
            let addr = addr.trim().to_string();
            if addr != "127.0.0.1" && !out.contains(&addr) {
                out.push(addr);
            }
        }
    }
    out
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = s.split('.');
    for o in octets.iter_mut() {
        *o = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(octets)
}

/// The gateway everything without a better route goes to.
fn default_route() -> Option<(String, String)> {
    let routes = read_trim("/proc/net/route")?;
    for line in routes.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 || cols[1] != "00000000" {
            continue;
        }
        // Little-endian hex, as the kernel writes it.
        let gw = u32::from_str_radix(cols[2], 16).ok()?.swap_bytes().to_be_bytes();
        return Some((
            cols[0].to_string(),
            format!("{}.{}.{}.{}", gw[0], gw[1], gw[2], gw[3]),
        ));
    }
    None
}

fn nameservers() -> Vec<String> {
    read_trim("/etc/resolv.conf")
        .map(|c| {
            c.lines()
                .filter_map(|l| l.trim().strip_prefix("nameserver "))
                .map(|s| s.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}
