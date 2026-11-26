//! The processor, from `/proc/cpuinfo` and `/sys/devices/system/cpu`.

use std::collections::BTreeSet;

use super::{read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    let info = read_trim("/proc/cpuinfo").unwrap_or_default();
    let first = |key: &str| -> Option<String> {
        info.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
    };

    // `model name` on x86; ARM has no such field, so the part number and
    // implementer are what identify it.
    let model = first("model name")
        .or_else(|| first("Model"))
        .or_else(|| first("Hardware"))
        .or_else(|| first("CPU part"))
        .unwrap_or_default();
    f.say("cpu", "Model", model);
    f.say("cpu", "Vendor", first("vendor_id").unwrap_or_default());

    let logical = info.lines().filter(|l| l.starts_with("processor")).count();
    // Physical cores: the distinct (physical id, core id) pairs. Counting
    // `cpu cores` instead gives one socket's worth and calls it the machine.
    let mut cores: BTreeSet<(String, String)> = BTreeSet::new();
    let mut pkg = String::new();
    for line in info.lines() {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim() {
                // `physical id` always precedes the `core id` of the same
                // processor block, so the socket in hand is the right one.
                "physical id" => pkg = v.trim().to_string(),
                "core id" => {
                    cores.insert((pkg.clone(), v.trim().to_string()));
                }
                _ => {}
            }
        }
    }
    let sockets = cores.iter().map(|(p, _)| p).collect::<BTreeSet<_>>().len();
    if logical > 0 {
        f.say(
            "cpu",
            "Cores",
            match (cores.len(), sockets) {
                (0, _) => format!("{logical} logical"),
                (c, s) if s > 1 => format!("{c} physical, {logical} logical, {s} sockets"),
                (c, _) => format!("{c} physical, {logical} logical"),
            },
        );
    }

    if let Some(mhz) = max_mhz() {
        f.say("cpu", "Max frequency", format!("{:.2} GHz", mhz / 1000.0));
    }
    f.say("cpu", "Cache", cache_sizes());
    f.say("cpu", "Microcode", first("microcode").unwrap_or_default());

    // Whether this machine can host virtual machines — the flag, not the
    // BIOS switch, which DMI does not expose.
    let flags = first("flags").or_else(|| first("Features")).unwrap_or_default();
    let virt = if flags.split_whitespace().any(|x| x == "vmx") {
        "Intel VT-x"
    } else if flags.split_whitespace().any(|x| x == "svm") {
        "AMD-V"
    } else {
        ""
    };
    f.say("cpu", "Virtualization", virt);
}

/// The highest clock any core is allowed, in MHz.
///
/// `cpuinfo_max_freq` rather than the `cpu MHz` line: that one is whatever the
/// core happened to be doing when the file was read, which on an idle machine
/// reports a laptop's 4.8 GHz part as an 800 MHz one.
fn max_mhz() -> Option<f64> {
    let khz = read_trim("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")?;
    khz.parse::<f64>().ok().map(|k| k / 1000.0)
}

/// The cache levels as `L1d 48K · L2 2M · L3 24M`, straight from sysfs.
fn cache_sizes() -> String {
    let mut out: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/devices/system/cpu/cpu0/cache") else {
        return String::new();
    };
    let mut items: Vec<(String, String)> = Vec::new();
    for e in entries.flatten() {
        let dir = e.path();
        let (Some(level), Some(size)) = (read_trim(dir.join("level")), read_trim(dir.join("size")))
        else {
            continue;
        };
        let kind = read_trim(dir.join("type")).unwrap_or_default();
        let name = match kind.as_str() {
            "Data" => format!("L{level}d"),
            "Instruction" => format!("L{level}i"),
            _ => format!("L{level}"),
        };
        items.push((name, size));
    }
    items.sort();
    for (name, size) in items {
        out.push(format!("{name} {size}"));
    }
    out.join(" · ")
}
