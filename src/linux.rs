//! What this machine is, read from `/proc`, `/sys` and `/etc`.
//!
//! Each area has its own submodule with a `collect(&mut Facts)`; this module
//! fans out to them in the order somebody reads an answer: what the machine is,
//! then what it runs, then what it is made of.
//!
//! Nothing here spawns a process. `dmidecode`, `lspci`, `lsblk` and `ip` all
//! read the same files this does, and each is a dependency that may be missing,
//! may be a different version, and — on a machine being triaged — may not be
//! the binary it claims to be.

use limen_sdk_rust::{json, Value};

mod cpu;
mod display;
mod firmware;
mod memory;
mod net;
pub(crate) mod os;
mod security;
mod storage;
mod system;

/// One `section | item | value` line, and the notes the collectors raise.
///
/// A fact rather than a row of a table: these are not a list of like things but
/// one machine described from several angles, and what identifies it lives in a
/// different place from what it is made of.
#[derive(Default)]
pub struct Facts {
    rows: Vec<Value>,
    /// What could not be read, and why — kept apart from the facts so an
    /// unreadable field never renders as an empty one.
    notes: Vec<String>,
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
}

impl Facts {
    /// The notes raised while collecting, for a test to read.
    #[cfg(test)]
    pub fn take_notes(self) -> Vec<String> {
        self.notes
    }
}

/// Collapse the whitespace sysfs pads its strings with, and drop the separator
/// the flat format reserves.
fn tidy(s: &str) -> String {
    s.replace('|', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn collect() -> Value {
    let mut f = Facts::default();
    system::collect(&mut f);
    os::collect(&mut f);
    cpu::collect(&mut f);
    memory::collect(&mut f);
    storage::collect(&mut f);
    net::collect(&mut f);
    display::collect(&mut f);
    firmware::collect(&mut f);
    security::collect(&mut f);

    json!({
        "os": "linux",
        "note": "What this machine is, read from /proc, /sys and /etc. \
                 `facts` are section/item/value; `notes` say which fields could \
                 not be read, which is not the same as a field that is empty.",
        "total": f.rows.len(),
        "facts": f.rows,
        "notes": f.notes,
    })
}

/// The DMI attributes that are `0400 root`, and where each belongs once read.
///
/// Named here rather than in the view because the list is the same thing twice
/// over: what [`system`] and [`firmware`] report as unreadable, and what the
/// elevated copy asks for.
pub const ROOT_ONLY: [(&str, &str, &str); 4] = [
    ("product_serial", "system", "Serial number"),
    ("product_uuid", "system", "UUID"),
    ("chassis_serial", "system", "Chassis serial"),
    ("board_serial", "firmware", "Board serial"),
];

/// The absolute paths of those attributes.
pub fn root_only_paths() -> Vec<String> {
    ROOT_ONLY
        .iter()
        .map(|(name, _, _)| format!("/sys/class/dmi/id/{name}"))
        .filter(|p| std::path::Path::new(p).exists())
        .collect()
}

/// Fold what an elevated copy recovered into a scan made without privileges.
///
/// Replaces the rows it can answer and drops the note that said they were
/// unreadable — a report that still carries "could not be read" beside the
/// value it now has is a report nobody trusts.
pub fn merge_elevated(scan: &mut Value, dir: &std::path::Path) -> usize {
    let mut recovered = 0;
    {
        let Some(facts) = scan.get_mut("facts").and_then(|f| f.as_array_mut()) else {
            return 0;
        };
        for (name, section, item) in ROOT_ONLY {
            let Some(value) = read_trim(dir.join(name)).map(|v| tidy(&v)) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            recovered += 1;
            // A field appended here lands at the end of its own section, which
            // is where the view puts it: sections are ordered, rows inside one
            // are shown as collected.
            match facts
                .iter_mut()
                .find(|f| f["section"] == section && f["item"] == item)
            {
                Some(existing) => existing["value"] = json!(value),
                None => facts.push(json!({
                    "section": section, "item": item, "value": value,
                })),
            }
        }
    }
    if recovered > 0 {
        if let Some(notes) = scan.get_mut("notes").and_then(|n| n.as_array_mut()) {
            notes.retain(|n| !n.as_str().unwrap_or("").starts_with("note.dmi_root"));
        }
        if let Some(total) = scan.get("facts").and_then(Value::as_array).map(|f| f.len()) {
            scan["total"] = json!(total);
        }
    }
    recovered
}

// ---- shared readers (the collectors reach them through `super::`) ---------- //

/// Read a file, trimmed; `None` if missing, unreadable or empty.
pub(crate) fn read_trim(path: impl AsRef<std::path::Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A DMI attribute. The serials among these are `0400 root` on most systems, so
/// an ordinary user gets `None` — which is why [`system`] says so out loud
/// rather than leaving the row blank.
pub(crate) fn dmi(name: &str) -> Option<String> {
    read_trim(format!("/sys/class/dmi/id/{name}"))
        // Vendors ship placeholder strings in DMI more often than they ship
        // real ones; a board called "To be filled by O.E.M." is not an answer.
        .filter(|v| {
            let l = v.to_ascii_lowercase();
            !(l.contains("to be filled")
                || l.contains("system manufacturer")
                || l.contains("default string")
                || l == "none"
                || l == "n/a"
                || l.chars().all(|c| c == '0' || c == '.' || c == ' '))
        })
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
