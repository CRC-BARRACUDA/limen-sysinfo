//! Memory: how much the system has and is using, and the modules it is made of.

use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use super::smbios::{self, Structure};
use super::Facts;
use crate::facts::human_bytes;

pub(super) fn collect(f: &mut Facts, table: &[Structure]) {
    // SAFETY: the structure is zeroed, says its own size as the call requires,
    // and is written only by the call.
    let status = unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        (GlobalMemoryStatusEx(&mut status) != 0).then_some(status)
    };
    if let Some(s) = status {
        let (total, available) = (s.ullTotalPhys, s.ullAvailPhys);
        f.say("memory", "Total", human_bytes(total));
        let used = total.saturating_sub(available);
        let pct = used.checked_mul(100).and_then(|u| u.checked_div(total)).unwrap_or(0);
        f.say("memory", "Used", format!("{} of {} ({pct}%)", human_bytes(used), human_bytes(total)));
        f.say("memory", "Available", human_bytes(available));
        // What Windows may promise beyond the memory it has is the page file.
        // How much of *that* is in use it does not say — what it counts is
        // memory promised, wherever it ends up living — so only the size is
        // given.
        let page_file = s.ullTotalPageFile.saturating_sub(total);
        f.say("memory", "Page file", if page_file == 0 { "none".into() } else { human_bytes(page_file) });
    }

    for module in table.iter().filter(|s| s.kind == smbios::MEMORY_DEVICE) {
        let Some(bytes) = module_bytes(module) else {
            continue; // an empty slot
        };
        let mut what = human_bytes(bytes);
        if let Some(kind) = module.byte(0x12).and_then(memory_type) {
            what = format!("{what} {kind}");
        }
        let mut parts = vec![what];
        if let Some(mhz) = module.word(0x15).filter(|&s| s != 0 && s != 0xFFFF) {
            parts.push(format!("{mhz} MHz"));
        }
        let made = format!("{} {}", module.string(0x17), module.string(0x1A));
        if !made.trim().is_empty() {
            parts.push(made.trim().to_string());
        }
        let slot = module.string(0x10);
        f.say("memory", if slot.is_empty() { "Module" } else { &slot }, parts.join(", "));
    }
}

/// The size of the module in a slot; `None` for a slot with nothing in it.
///
/// One 16-bit field that outgrew itself: megabytes, or kilobytes if the top bit
/// is set, or — at `0x7FFF` — "see the 32-bit field added for modules of 32 GB
/// and up".
pub(crate) fn module_bytes(module: &Structure) -> Option<u64> {
    const MB: u64 = 1_048_576;
    match module.word(0x0C)? {
        0 | 0xFFFF => None, // empty, or the firmware does not know
        0x7FFF => module.dword(0x1C).map(|mb| u64::from(mb & 0x7FFF_FFFF) * MB),
        kb if kb & 0x8000 != 0 => Some(u64::from(kb & 0x7FFF) * 1024),
        mb => Some(u64::from(mb) * MB),
    }
}

/// The generation of a module, for the ones still found in machines.
fn memory_type(code: u8) -> Option<&'static str> {
    Some(match code {
        0x12 => "DDR",
        0x13 => "DDR2",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        _ => return None,
    })
}
