//! What the machine is protected by.
//!
//! Only what an ordinary user can be told and what the system states outright.
//! Whether a drive is BitLocker-encrypted, and whether an antivirus is running,
//! are deliberately not here: Windows answers the first only to an
//! administrator, and the second lives in places where "the value is absent"
//! and "the protection is off" look the same. A wrong line under "Security" is
//! worse than a missing one.

use windows_sys::Win32::System::TpmBaseServices::{Tbsi_GetDeviceInfo, TPM_DEVICE_INFO};

use super::{reg_u32, Facts};

pub(super) fn collect(f: &mut Facts) {
    f.say("security", "Secure Boot", secure_boot());
    f.say("security", "TPM", tpm());
    f.say("security", "UAC", uac());
}

/// Whether Secure Boot was on when this Windows booted.
///
/// Windows writes what the firmware told it here on every boot. The key is
/// absent on a machine booted the legacy way — `firmware.Type` already says so.
fn secure_boot() -> String {
    match reg_u32(r"SYSTEM\CurrentControlSet\Control\SecureBoot\State", "UEFISecureBootEnabled") {
        Some(0) => "disabled".into(),
        Some(_) => "enabled".into(),
        None => String::new(),
    }
}

/// A TPM, and which specification it implements — 1.2 and 2.0 are different
/// enough that "has a TPM" is not an answer on its own.
fn tpm() -> String {
    const NOT_FOUND: u32 = 0x8028_400F; // TBS_E_TPM_NOT_FOUND
    // SAFETY: the structure is given with its own size and written only by
    // the call.
    let (result, info) = unsafe {
        let mut info: TPM_DEVICE_INFO = std::mem::zeroed();
        let result = Tbsi_GetDeviceInfo(
            std::mem::size_of::<TPM_DEVICE_INFO>() as u32,
            (&mut info as *mut TPM_DEVICE_INFO).cast(),
        );
        (result, info)
    };
    match (result, info.tpmVersion) {
        (0, 1) => "TPM 1.2".into(),
        (0, 2) => "TPM 2.0".into(),
        (NOT_FOUND, _) => "none".into(),
        // The service that answers is switched off, or said something new:
        // either way that is not "this machine has no TPM".
        _ => String::new(),
    }
}

/// Whether programs must ask before acting as administrator.
///
/// Off, every program an administrator starts has full rights from the first
/// instruction — which changes what a single careless click was able to do.
fn uac() -> String {
    match reg_u32(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System", "EnableLUA") {
        Some(0) => "disabled".into(),
        Some(_) => "enabled".into(),
        None => String::new(),
    }
}
