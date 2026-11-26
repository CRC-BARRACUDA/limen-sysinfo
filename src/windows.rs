//! What this machine is, asked of Windows itself.
//!
//! Linux answers from files; Windows keeps the same answers in three places,
//! and each is read here directly:
//!
//! - the **firmware's own table** (SMBIOS, through `GetSystemFirmwareTable`) —
//!   who made the machine, its serial, its BIOS, its memory modules. Windows
//!   hands it to an ordinary user, so nothing here needs administrator rights;
//! - the **registry** — which Windows this is, and what the processor calls
//!   itself;
//! - a handful of **system calls** — memory, drives, network adapters, screens.
//!
//! Nothing here spawns a process and nothing goes through WMI: `wmic`,
//! `systeminfo` and PowerShell would all answer, and each is a program that may
//! be missing, blocked by policy, or — on a machine being triaged — not the
//! binary it claims to be.
//!
//! The sections and item names match the Linux collector wherever the two
//! systems mean the same thing, so a fleet answer and a local one agree.
//!
//! ```text
//!   smbios    the firmware's table, parsed
//!   system    who made it, which one it is
//!   os        which Windows, since when
//!   cpu       the processor
//!   memory    how much, and the modules it is made of
//!   storage   drives and volumes
//!   net       adapters, the default route, DNS
//!   display   graphics adapters and screens
//!   firmware  BIOS and board
//!   security  Secure Boot, TPM, UAC
//! ```

use limen_sdk_rust::{json, Value};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub(crate) use crate::facts::Facts;

pub(crate) mod cpu;
pub(crate) mod display;
mod firmware;
pub(crate) mod memory;
mod net;
pub(crate) mod os;
mod security;
pub(crate) mod smbios;
pub(crate) mod storage;
mod system;

pub fn collect() -> Value {
    let table = smbios::read();
    let mut f = Facts::default();
    system::collect(&mut f, &table);
    os::collect(&mut f);
    cpu::collect(&mut f);
    memory::collect(&mut f, &table);
    storage::collect(&mut f);
    net::collect(&mut f);
    display::collect(&mut f);
    firmware::collect(&mut f, &table);
    security::collect(&mut f);

    json!({
        "os": "windows",
        "note": "What this machine is, read from the firmware table, the \
                 registry and the system's own calls. `facts` are \
                 section/item/value; `notes` say which fields could not be \
                 read, which is not the same as a field that is empty.",
        "total": f.rows.len(),
        "facts": f.rows,
        "notes": f.notes,
    })
}

// ---- shared readers (the collectors reach them through `super::`) ---------- //

/// `s` as the zero-ended UTF-16 the system's calls take.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The text in a UTF-16 buffer, up to its first zero.
pub(crate) fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// The text a system call left a pointer to.
///
/// # Safety
/// `p` is null, or points at UTF-16 that ends with a zero.
pub(crate) unsafe fn from_pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    // No name the system hands out is longer; the bound is against a pointer
    // that is not what it should be.
    while len < 4096 && *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

/// One value under `HKEY_LOCAL_MACHINE`, as the bytes the registry holds —
/// `None` when it is absent, unreadable, or of another type than `flags` asks.
fn reg_raw(path: &str, name: &str, flags: u32) -> Option<Vec<u8>> {
    let (path, name) = (wide(path), wide(name));
    let mut size: u32 = 0;
    // SAFETY: both names end with a zero; with no buffer the call only reports
    // the size, and with one it writes at most the `size` it was given.
    unsafe {
        let asked = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            name.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        );
        if asked != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let read = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            name.as_ptr(),
            flags,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        );
        if read != 0 {
            return None;
        }
        buf.truncate(size as usize);
        Some(buf)
    }
}

/// A text value under `HKEY_LOCAL_MACHINE`, trimmed; `None` if absent or empty.
pub(crate) fn reg_string(path: &str, name: &str) -> Option<String> {
    let raw = reg_raw(path, name, RRF_RT_REG_SZ)?;
    let units: Vec<u16> = raw.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    Some(from_wide(&units).trim().to_string()).filter(|s| !s.is_empty())
}

/// A number under `HKEY_LOCAL_MACHINE`.
pub(crate) fn reg_u32(path: &str, name: &str) -> Option<u32> {
    let raw = reg_raw(path, name, RRF_RT_REG_DWORD)?;
    Some(u32::from_le_bytes(raw.get(..4)?.try_into().ok()?))
}

/// A binary value under `HKEY_LOCAL_MACHINE`.
pub(crate) fn reg_bytes(path: &str, name: &str) -> Option<Vec<u8>> {
    reg_raw(path, name, RRF_RT_REG_BINARY)
}
