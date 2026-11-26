//! The firmware's own description of the machine — the SMBIOS table.
//!
//! The same table `dmidecode` prints and `/sys/class/dmi` is cut from. Windows
//! hands the whole of it to any process that asks, serials included.
//!
//! It is a run of structures. Each starts with its type, the length of its
//! fixed part and a handle; the fixed part refers to its strings by number, and
//! the strings follow it, each ended by a zero and the set by one more.

use windows_sys::Win32::System::SystemInformation::GetSystemFirmwareTable;

use crate::facts::is_placeholder;

/// `'RSMB'` — the raw SMBIOS table, among the firmware tables there are.
const RSMB: u32 = 0x5253_4D42;

pub(crate) const BIOS: u8 = 0;
pub(crate) const SYSTEM: u8 = 1;
pub(crate) const BOARD: u8 = 2;
pub(crate) const CHASSIS: u8 = 3;
pub(crate) const MEMORY_DEVICE: u8 = 17;
const END: u8 = 127;

/// One structure of the table: its fixed part, and the strings that part
/// refers to.
pub(crate) struct Structure {
    pub kind: u8,
    data: Vec<u8>,
    strings: Vec<String>,
}

impl Structure {
    pub fn byte(&self, at: usize) -> Option<u8> {
        self.data.get(at).copied()
    }

    pub fn word(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.data.get(at..at + 2)?.try_into().ok()?))
    }

    pub fn dword(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.data.get(at..at + 4)?.try_into().ok()?))
    }

    pub fn bytes(&self, at: usize, len: usize) -> Option<&[u8]> {
        self.data.get(at..at + len)
    }

    /// The string the field at `at` names, or nothing: when the structure is
    /// too old to have the field, when the field names no string, and when the
    /// string is a vendor's placeholder rather than an answer.
    pub fn string(&self, at: usize) -> String {
        let Some(number) = self.byte(at).filter(|&n| n > 0) else {
            return String::new();
        };
        self.strings
            .get(number as usize - 1)
            .map(|s| s.trim().to_string())
            .filter(|s| !is_placeholder(s))
            .unwrap_or_default()
    }
}

/// The first structure of a kind — there is one BIOS and one system.
pub(crate) fn first(table: &[Structure], kind: u8) -> Option<&Structure> {
    table.iter().find(|s| s.kind == kind)
}

/// The machine's table, or nothing if the firmware gives none (which is what a
/// machine without SMBIOS answers, and some virtual ones are).
pub(crate) fn read() -> Vec<Structure> {
    // SAFETY: with no buffer the call only reports the size; with one it
    // writes at most the size it was given.
    let raw = unsafe {
        let size = GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0);
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let got = GetSystemFirmwareTable(RSMB, 0, buf.as_mut_ptr().cast(), size);
        if got == 0 || got > size {
            return Vec::new();
        }
        buf.truncate(got as usize);
        buf
    };
    // What Windows returns is the table behind eight bytes of its own: the
    // SMBIOS version, and at 4 the length of what follows.
    let Some(len) = raw.get(4..8).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else {
        return Vec::new();
    };
    let end = (8 + len as usize).min(raw.len());
    parse(&raw[8..end])
}

/// Cut a table into its structures. A table that ends mid-structure gives the
/// structures before the break.
pub(crate) fn parse(table: &[u8]) -> Vec<Structure> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 4 <= table.len() {
        let kind = table[at];
        let len = table[at + 1] as usize;
        if len < 4 || at + len > table.len() {
            break;
        }
        let data = table[at..at + len].to_vec();

        let mut strings = Vec::new();
        let mut s = at + len;
        loop {
            let Some(end) = table.get(s..).and_then(|rest| rest.iter().position(|&b| b == 0))
            else {
                return out; // the strings run off the end of the table
            };
            if end == 0 {
                // The zero that ends the set. A structure with no strings at
                // all still has two of them.
                s += if strings.is_empty() { 2 } else { 1 };
                break;
            }
            strings.push(String::from_utf8_lossy(&table[s..s + end]).into_owned());
            s += end + 1;
        }
        at = s;

        if kind == END {
            break;
        }
        out.push(Structure { kind, data, strings });
    }
    out
}

/// The system UUID as it is written everywhere else.
///
/// Sixteen bytes, of which the first three groups are stored low byte first —
/// printing them in the order they are stored gives a UUID that matches
/// nothing: not the sticker, not `dmidecode`, not the inventory. All zeros and
/// all ones both mean the vendor set none.
pub(crate) fn uuid(raw: &[u8]) -> Option<String> {
    let d: &[u8; 16] = raw.try_into().ok()?;
    if d.iter().all(|&b| b == 0) || d.iter().all(|&b| b == 0xFF) {
        return None;
    }
    Some(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        d[3], d[2], d[1], d[0], d[5], d[4], d[7], d[6], d[8], d[9], d[10], d[11], d[12], d[13],
        d[14], d[15],
    ))
}
