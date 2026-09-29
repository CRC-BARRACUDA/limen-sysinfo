//! What the machine is protected by — the four answers that change how much a
//! disk image is worth and how a compromise could have happened.

use super::{read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    f.say("security", "Secure Boot", secure_boot());
    f.say("security", "TPM", tpm());
    f.say("security", "Mandatory access control", mac());
    f.say("security", "Kernel lockdown", lockdown());
}

/// Whether Secure Boot was on when this kernel booted.
///
/// The EFI variable is raw: five bytes, of which the last is the flag. Reading
/// it as text gives a byte that is not a digit, which is why it is read as
/// bytes and indexed rather than parsed.
fn secure_boot() -> String {
    let Ok(dir) = std::fs::read_dir("/sys/firmware/efi/efivars") else {
        return String::new(); // not a UEFI boot — `firmware.Type` already says so
    };
    let Some(var) = dir
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("SecureBoot-"))
    else {
        return String::new();
    };
    // Readable only by root on many systems: say nothing rather than report a
    // machine with Secure Boot on as having none.
    let Ok(bytes) = std::fs::read(var.path()) else {
        return String::new();
    };
    match bytes.last() {
        Some(1) => "enabled".into(),
        Some(0) => "disabled".into(),
        _ => String::new(),
    }
}

/// A TPM, and which specification it implements — 1.2 and 2.0 are different
/// enough that "has a TPM" is not an answer on its own.
fn tpm() -> String {
    let Ok(entries) = std::fs::read_dir("/sys/class/tpm") else {
        return "none".into();
    };
    // The first chip there is: a machine with two TPMs is a machine with a
    // TPM, and which one answered is not the question.
    let Some(entry) = entries.flatten().next() else {
        return "none".into();
    };
    let name = entry.file_name().to_string_lossy().to_string();
    match read_trim(entry.path().join("tpm_version_major")) {
        Some(major) => format!("{name}, TPM {major}.0"),
        // Pre-2.0 chips have no version file; the device node is the evidence.
        None => format!("{name}, TPM 1.2"),
    }
}

/// SELinux or AppArmor, and whether it is actually enforcing.
///
/// "Installed" and "enforcing" are different states, and a permissive SELinux
/// logs what it would have stopped rather than stopping it.
fn mac() -> String {
    if let Some(mode) = read_trim("/sys/fs/selinux/enforce") {
        return match mode.as_str() {
            "1" => "SELinux, enforcing".into(),
            "0" => "SELinux, permissive".into(),
            _ => "SELinux".into(),
        };
    }
    if std::path::Path::new("/sys/kernel/security/apparmor").exists() {
        let profiles = read_trim("/sys/kernel/security/apparmor/profiles")
            .map(|p| p.lines().count())
            .unwrap_or(0);
        return if profiles > 0 {
            format!("AppArmor, {profiles} profiles")
        } else {
            "AppArmor".into()
        };
    }
    "none".into()
}

/// Kernel lockdown, which decides whether root can read kernel memory or load
/// unsigned modules — the difference between a compromised root and a
/// compromised kernel.
fn lockdown() -> String {
    let Some(raw) = read_trim("/sys/kernel/security/lockdown") else {
        return String::new();
    };
    // `none [integrity] confidentiality` — the active one is in brackets.
    raw.split_whitespace()
        .find(|w| w.starts_with('['))
        .map(|w| w.trim_matches(['[', ']']).to_string())
        .unwrap_or_default()
}
