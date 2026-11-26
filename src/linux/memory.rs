//! Memory and swap, from `/proc/meminfo`.

use super::{human_bytes, read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    let Some(info) = read_trim("/proc/meminfo") else {
        return;
    };
    let kb = |key: &str| -> Option<u64> {
        info.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
    };
    let bytes = |key: &str| kb(key).map(|v| v * 1024);

    let total = bytes("MemTotal:");
    let available = bytes("MemAvailable:");
    if let Some(t) = total {
        f.say("memory", "Total", human_bytes(t));
        // "Used" as the machine experiences it: total minus what a new process
        // could actually get. `MemFree` alone counts the page cache as used and
        // reports every healthy Linux box as nearly full.
        if let Some(a) = available {
            let used = t.saturating_sub(a);
            let pct = used.checked_mul(100).and_then(|u| u.checked_div(t)).unwrap_or(0);
            f.say("memory", "Used", format!("{} of {} ({pct}%)", human_bytes(used), human_bytes(t)));
            f.say("memory", "Available", human_bytes(a));
        }
    }
    if let (Some(st), Some(sf)) = (bytes("SwapTotal:"), bytes("SwapFree:")) {
        f.say(
            "memory",
            "Swap",
            if st == 0 {
                "none".to_string()
            } else {
                format!("{} of {} used", human_bytes(st - sf), human_bytes(st))
            },
        );
    }
}
