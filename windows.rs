//! Windows — not collected yet.
//!
//! Deliberately a stub rather than a half-written collector. The Linux side
//! reads files; the Windows answers live in WMI (`Win32_ComputerSystem`,
//! `Win32_BIOS`, `Win32_Processor`), the registry, and `GetSystemFirmwareTable`
//! — a different shape of code, and one worth writing on a Windows machine
//! where it can be run rather than guessed at.
//!
//! What this returns is the same envelope the Linux collector returns, with no
//! facts and a note saying why. That is the honest answer: an empty list of
//! facts on its own reads as "this machine has nothing to report", and the
//! whole point of the module is to say what a machine is.
//!
//! The sections to fill, matching Linux and `crowdstrike-sysinfo` so a fleet
//! answer and a local one agree:
//!
//! | section | source |
//! |---|---|
//! | system | `Win32_ComputerSystem`, `Win32_ComputerSystemProduct` (UUID, serial) |
//! | os | `Win32_OperatingSystem`, `CurrentVersion` registry (build, UBR) |
//! | cpu | `Win32_Processor` |
//! | memory | `Win32_PhysicalMemory`, `GlobalMemoryStatusEx` |
//! | disk | `Win32_DiskDrive`, `Win32_LogicalDisk` |
//! | network | `Win32_NetworkAdapterConfiguration` |
//! | display | `Win32_VideoController`, `Win32_DesktopMonitor` |
//! | firmware | `Win32_BIOS`, `Win32_BaseBoard`, UEFI via `GetFirmwareType` |
//! | security | Secure Boot (`UEFISecureBootEnabled`), BitLocker, TPM (`Win32_Tpm`) |

use limen_sdk_rust::{json, Value};

pub fn collect() -> Value {
    json!({
        "os": "windows",
        "note": "The Windows collector is not written yet.",
        "total": 0,
        "facts": [],
        "notes": ["note.no_collector\u{1f}windows"],
    })
}
