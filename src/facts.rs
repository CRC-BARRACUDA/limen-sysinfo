//! What every collector says its answers in, whatever it read them from.
//!
//! The facts themselves, and the handful of readings that are the same on any
//! machine: how a size or a duration is said, which DMI strings are a vendor's
//! placeholder rather than an answer, what a chassis number means, and which
//! hypervisor a system's name gives away.

use limen_sdk_rust::{json, Value};

/// One `section | item | value` line, and the notes the collectors raise.
///
/// A fact rather than a row of a table: these are not a list of like things but
/// one machine described from several angles, and what identifies it lives in a
/// different place from what it is made of.
#[derive(Default)]
pub struct Facts {
    pub(crate) rows: Vec<Value>,
    /// What could not be read, and why — kept apart from the facts so an
    /// unreadable field never renders as an empty one.
    pub(crate) notes: Vec<String>,
}

impl Facts {
    /// Record `value` unless it is empty — a fact nobody can read is not a fact,
    /// and a blank row reads as "this machine has none of that".
    pub fn say(&mut self, section: &str, item: &str, value: impl AsRef<str>) {
        let value = tidy(value.as_ref());
        if value.is_empty() {
            return;
        }
        self.rows.push(json!({
            "section": section,
            "item": item,
            "value": value,
        }));
    }

    /// A key that was asked for and could not be answered — the difference
    /// between "this machine has no serial" and "this process may not read it".
    ///
    /// Raised twice for the same reason, the fields are gathered into one note:
    /// the sentence explaining that DMI serials need root is worth reading once,
    /// however many fields it cost.
    #[allow(dead_code)] // nothing on Windows needs asking for twice
    pub fn note(&mut self, key: &str, arg: impl AsRef<str>) {
        let arg = arg.as_ref();
        if arg.is_empty() {
            if !self.notes.iter().any(|n| n == key) {
                self.notes.push(key.to_string());
            }
            return;
        }
        let prefix = format!("{key}\u{1f}");
        if let Some(existing) = self.notes.iter_mut().find(|n| n.starts_with(&prefix)) {
            if !existing.split(&prefix).nth(1).is_some_and(|a| a.split(", ").any(|f| f == arg)) {
                existing.push_str(", ");
                existing.push_str(arg);
            }
            return;
        }
        self.notes.push(format!("{prefix}{arg}"));
    }

    /// The notes raised while collecting, for a test to read.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn take_notes(self) -> Vec<String> {
        self.notes
    }
}

/// Collapse the whitespace firmware pads its strings with, and drop the
/// separator the flat format reserves.
pub(crate) fn tidy(s: &str) -> String {
    s.replace('|', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bytes as the size a person would say: `931.5 GB`, `16.0 GB`, `512 MB`.
pub(crate) fn human_bytes(bytes: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{bytes} B")
    }
}

/// Seconds as `12d 4h 37m`, dropping the units that are zero at the front.
pub(crate) fn human_duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h {m}m"),
    }
}

/// A Unix timestamp as `YYYY-MM-DD HH:MM UTC`.
///
/// UTC, deliberately: the local offset would have to be guessed from the
/// timezone name, and a boot time that is right in one reading and wrong in
/// another is worse than one that says which it is.
pub(crate) fn format_unix_utc(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm), which is exact and needs no
    // table of month lengths.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60
    )
}

/// Whether a DMI string is what a vendor left in the field instead of filling
/// it in.
///
/// Vendors ship placeholder strings in DMI more often than they ship real
/// ones; a board called "To be filled by O.E.M." is not an answer.
pub(crate) fn is_placeholder(v: &str) -> bool {
    let l = v.trim().to_ascii_lowercase();
    l.is_empty()
        || l.contains("to be filled")
        || l.contains("system manufacturer")
        || l.contains("default string")
        || l == "none"
        || l == "n/a"
        || l.chars().all(|c| c == '0' || c == '.' || c == ' ')
}

/// The DMI chassis type, as a word rather than the number SMBIOS stores.
///
/// Worth having in an inventory: "this serial is a laptop" answers a different
/// question from "this serial is a rack server", and the two are handled by
/// different people.
pub(crate) fn chassis_name(code: u8) -> String {
    match code {
        1 => "Other",
        2 => "Unknown",
        3 | 4 | 6 | 7 | 15 => "Desktop",
        5 => "Pizza box",
        8 | 9 | 10 | 14 => "Laptop",
        11 => "Hand held",
        12 => "Docking station",
        13 => "All in one",
        16 => "Lunch box",
        17 | 23 | 28 => "Server",
        18 => "Expansion chassis",
        21 => "Peripheral chassis",
        22 => "RAID chassis",
        24 => "Sealed-case PC",
        30 => "Tablet",
        31 => "Convertible",
        32 => "Detachable",
        other => return format!("Type {other}"),
    }
    .to_string()
}

/// The hypervisor a guest is running on, when DMI names it outright — which it
/// usually does: a virtual machine's "manufacturer" is whoever virtualized it.
pub(crate) fn hypervisor_guest(vendor: &str, product: &str) -> Option<String> {
    let both = format!("{} {}", vendor.to_ascii_lowercase(), product.to_ascii_lowercase());
    for (marker, name) in [
        ("vmware", "VMware"),
        ("virtualbox", "VirtualBox"),
        ("innotek", "VirtualBox"),
        ("qemu", "QEMU/KVM"),
        ("kvm", "QEMU/KVM"),
        ("bochs", "QEMU"),
        ("xen", "Xen"),
        ("microsoft corporation virtual", "Hyper-V"),
        ("parallels", "Parallels"),
        ("amazon ec2", "Amazon EC2"),
        ("google", "Google Compute Engine"),
        ("alibaba", "Alibaba Cloud"),
        ("openstack", "OpenStack"),
    ] {
        if both.contains(marker) {
            return Some(format!("{name} guest"));
        }
    }
    None
}
