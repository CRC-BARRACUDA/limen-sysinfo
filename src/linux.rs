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

pub(crate) use crate::facts::{human_bytes, human_duration, Facts};
use crate::facts::{is_placeholder, tidy};

mod cpu;
mod display;
mod firmware;
mod memory;
mod net;
pub(crate) mod os;
mod security;
mod storage;
mod system;

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
        .filter(|v| !is_placeholder(v))
}
