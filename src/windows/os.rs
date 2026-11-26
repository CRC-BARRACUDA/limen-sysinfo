//! Which Windows this is, and since when it has been running.

use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

use super::{from_wide, reg_string, reg_u32, Facts};
use crate::facts::{format_unix_utc, human_duration};

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

pub(super) fn collect(f: &mut Facts) {
    let cv = |name: &str| reg_string(CURRENT_VERSION, name);
    let build = cv("CurrentBuild").unwrap_or_default();

    f.say("os", "Name", product_name(&cv("ProductName").unwrap_or_default(), &build));
    // `DisplayVersion` is the "24H2" people say; `ReleaseId` is what it was
    // called before 20H2 and is frozen at 2009 since.
    f.say("os", "Version", cv("DisplayVersion").or_else(|| cv("ReleaseId")).unwrap_or_default());
    // The build with its update revision: the first half says which Windows,
    // the second which month's patches are on it.
    match reg_u32(CURRENT_VERSION, "UBR") {
        Some(ubr) if !build.is_empty() => f.say("os", "Build", format!("{build}.{ubr}")),
        _ => f.say("os", "Build", &build),
    }
    if let (Some(major), Some(minor)) = (
        reg_u32(CURRENT_VERSION, "CurrentMajorVersionNumber"),
        reg_u32(CURRENT_VERSION, "CurrentMinorVersionNumber"),
    ) {
        f.say("os", "Kernel", format!("NT {major}.{minor}.{build}"));
    } else {
        // Before Windows 10 the version was one string.
        f.say("os", "Kernel", cv("CurrentVersion").map(|v| format!("NT {v}.{build}")).unwrap_or_default());
    }
    f.say("os", "Architecture", std::env::consts::ARCH);

    // SAFETY: takes nothing and cannot fail.
    let up = unsafe { GetTickCount64() } / 1000;
    f.say("os", "Uptime", human_duration(up));
    if let Some(now) = unix_now() {
        f.say("os", "Booted", format_unix_utc(now - up as i64));
    }
    if let Some(when) = reg_u32(CURRENT_VERSION, "InstallDate").filter(|&t| t > 0) {
        // Said with where it came from, because it is not quite what it is
        // called: Windows rewrites it when a feature update replaces the
        // system, so on a machine that has been upgraded this is the date of
        // the upgrade.
        f.say(
            "os",
            "Installed",
            format!(
                "{} (from the registry's InstallDate, which a feature update resets)",
                format_unix_utc(i64::from(when))
            ),
        );
    }
    f.say(
        "os",
        "Timezone",
        reg_string(r"SYSTEM\CurrentControlSet\Control\TimeZoneInformation", "TimeZoneKeyName")
            .unwrap_or_default(),
    );
    f.say("os", "Locale", locale());
}

/// The product name, corrected for the one thing the registry gets wrong.
///
/// Windows 11 still writes "Windows 10" there — the value was never updated,
/// and Microsoft's own tools go by the build number instead. 22000 is the first
/// build of Windows 11.
pub(crate) fn product_name(name: &str, build: &str) -> String {
    if build.parse::<u32>().is_ok_and(|b| b >= 22_000) {
        return name.replacen("Windows 10", "Windows 11", 1);
    }
    name.to_string()
}

fn unix_now() -> Option<i64> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(now.as_secs()).ok()
}

/// The user's locale as `uk-UA` — what `LANG` answers on Linux.
fn locale() -> String {
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    // SAFETY: the length given is the length of `buf`.
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 0 {
        return String::new();
    }
    from_wide(&buf)
}
