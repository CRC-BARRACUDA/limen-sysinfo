//! What the machine runs: the distribution, the kernel, and how long it has
//! been up — which is the first thing anybody asks of a host under triage.

use super::{human_duration, read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    let release = os_release();
    let field = |k: &str| release.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());

    f.say("os", "Name", field("PRETTY_NAME").or_else(|| field("NAME")).unwrap_or_default());
    f.say("os", "Version", field("VERSION").or_else(|| field("VERSION_ID")).unwrap_or_default());
    f.say("os", "Build", field("BUILD_ID").unwrap_or_default());
    f.say("os", "Kernel", kernel());
    f.say("os", "Architecture", std::env::consts::ARCH);
    f.say("os", "Init", init_system());

    if let Some(up) = uptime_secs() {
        f.say("os", "Uptime", human_duration(up));
    }
    if let Some(boot) = boot_time() {
        f.say("os", "Booted", boot);
    }
    if let Some((when, from)) = install_date() {
        f.say("os", "Installed", format!("{when} (from {from})"));
    }
    f.say("os", "Timezone", timezone());
    f.say("os", "Locale", std::env::var("LANG").unwrap_or_default());
    // The session the *user* is in, which is not the same question as what is
    // installed: a machine with three desktops has one in use.
    f.say("os", "Desktop", std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
    f.say("os", "Session type", std::env::var("XDG_SESSION_TYPE").unwrap_or_default());
}

/// `/etc/os-release` as name/value pairs, quotes stripped.
fn os_release() -> Vec<(String, String)> {
    let text = read_trim("/etc/os-release")
        .or_else(|| read_trim("/usr/lib/os-release"))
        .unwrap_or_default();
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect()
}

/// `Linux 6.11.3-200.fc40.x86_64`, the way `uname -sr` prints it — from
/// `/proc/sys/kernel`, which is where `uname` reads it from.
fn kernel() -> String {
    let name = read_trim("/proc/sys/kernel/ostype").unwrap_or_else(|| "Linux".into());
    match read_trim("/proc/sys/kernel/osrelease") {
        Some(rel) => format!("{name} {rel}"),
        None => name,
    }
}

fn uptime_secs() -> Option<u64> {
    read_trim("/proc/uptime")?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .map(|s| s as u64)
}

/// When the machine came up, as a local date and time.
///
/// Derived from `btime` in `/proc/stat` (a Unix timestamp) rather than read as
/// text anywhere, and formatted here because pulling in a date library for one
/// line is not worth the dependency.
fn boot_time() -> Option<String> {
    let stat = read_trim("/proc/stat")?;
    let btime: i64 = stat
        .lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    Some(format_unix_utc(btime))
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

/// When this installation was made, and what that answer is based on.
///
/// Linux records no installation date anywhere — there is no field to read, and
/// `rpm -qi basesystem` or `tune2fs -l` would both mean spawning a process this
/// module does not spawn. What there is instead is the **birth time** of the
/// files an installer creates and nothing afterwards rewrites.
///
/// The earliest birth time among those wins: a file edited later keeps the
/// birth time it was created with, so a rewritten `/etc/hostname` moves its
/// *modification* time and not this answer.
///
/// The source is reported with the date, because this is derived rather than
/// read, and an analyst putting a date in a report needs to know which file it
/// came from. Modification time is used only for `/etc/machine-id`, which is
/// written once at first boot; every other file here ships in a package, and
/// its mtime is the date that package was **built** — on this machine, two
/// months before the install.
fn install_date() -> Option<(String, &'static str)> {
    // In the order an installer touches them; ties go to the first.
    const CANDIDATES: [&str; 5] = [
        "/etc/machine-id",
        "/etc/fstab",
        "/var/log/installer",
        "/root",
        "/etc/hostname",
    ];
    let mut best: Option<(i64, &'static str)> = None;
    for path in CANDIDATES {
        let Ok(meta) = std::fs::metadata(path) else { continue };
        // `created()` is statx's `btime` on Linux, and `Err` on a filesystem
        // that does not keep one.
        // `continue`, not `?`: a candidate whose filesystem keeps no birth time
        // is one candidate less, not the end of the search.
        let Some(stamp) = meta
            .created()
            .ok()
            .or_else(|| (path == "/etc/machine-id").then(|| meta.modified().ok()).flatten())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
        else {
            continue;
        };
        // A clock that was wrong when the file was made, or a file from the
        // future, is not an installation date.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(i64::MAX);
        if stamp < 946_684_800 || stamp > now {
            continue;
        }
        if best.is_none_or(|(b, _)| stamp < b) {
            best = Some((stamp, path));
        }
    }
    best.map(|(stamp, path)| (format_unix_utc(stamp), path))
}

/// The init system, by name — `systemd`, `runit`, whatever PID 1 is.
fn init_system() -> String {
    read_trim("/proc/1/comm").unwrap_or_default()
}

/// The configured timezone: the symlink `/etc/localtime` points into the zone
/// database, and its tail is the name.
fn timezone() -> String {
    if let Some(tz) = read_trim("/etc/timezone") {
        return tz;
    }
    let Ok(target) = std::fs::read_link("/etc/localtime") else {
        return String::new();
    };
    let path = target.to_string_lossy();
    match path.split_once("zoneinfo/") {
        Some((_, name)) => name.to_string(),
        None => String::new(),
    }
}
