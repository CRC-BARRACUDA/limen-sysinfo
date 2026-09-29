//! Graphics adapters and the screens attached to them.

use super::{read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    gpus(f);
    screens(f);
}

/// PCI functions whose class byte says "display controller".
///
/// Named by vendor id rather than looked up in `pci.ids`: that file is a
/// package that may not be installed, and the vendor is the part that matters —
/// "which driver stack is this machine on" is answered by NVIDIA vs AMD vs
/// Intel, not by the exact model string.
fn gpus(f: &mut Facts) {
    let Ok(entries) = std::fs::read_dir("/sys/bus/pci/devices") else {
        return;
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let class = read_trim(dir.join("class"))
            .and_then(|c| u32::from_str_radix(c.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        if (class >> 16) & 0xff != 0x03 {
            continue;
        }
        let vendor_id = read_trim(dir.join("vendor")).unwrap_or_default();
        let device_id = read_trim(dir.join("device")).unwrap_or_default();
        let vendor = match vendor_id.trim_start_matches("0x") {
            "8086" => "Intel",
            "10de" => "NVIDIA",
            "1002" | "1022" => "AMD",
            "15ad" => "VMware",
            "1234" | "1b36" => "QEMU",
            "80ee" => "VirtualBox",
            "1414" => "Microsoft",
            "102b" => "Matrox",
            "1a03" => "ASPEED",
            other => other,
        };
        let driver = std::fs::read_link(dir.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let slot = entry.file_name().to_string_lossy().to_string();
        let mut value = format!("{vendor} [{}:{}]", vendor_id.trim_start_matches("0x"),
                                device_id.trim_start_matches("0x"));
        if !driver.is_empty() {
            value = format!("{value}, driver {driver}");
        }
        rows.push((slot, value));
    }
    rows.sort();
    for (slot, value) in rows {
        f.say("display", &format!("GPU {slot}"), value);
    }
}

/// Connected outputs and what they are running, from `/sys/class/drm`.
///
/// The first line of `modes` is the mode in use, so a connected screen reports
/// its resolution without asking the display server — which would mean talking
/// to X11 or Wayland, and being wrong on a headless machine.
fn screens(f: &mut Facts) {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return;
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if read_trim(dir.join("status")).as_deref() != Some("connected") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // `card1-DP-3` — the connector is the half worth reading.
        let label = name.split_once('-').map(|(_, c)| c.to_string()).unwrap_or(name);
        let mode = read_trim(dir.join("modes"))
            .and_then(|m| m.lines().next().map(str::to_string))
            .unwrap_or_default();
        rows.push((label, mode));
    }
    rows.sort();
    for (label, mode) in rows {
        f.say("display", &label, if mode.is_empty() { "connected".into() } else { mode });
    }
}
