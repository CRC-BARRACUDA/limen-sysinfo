//! Disks and the filesystems mounted on them.
//!
//! Two questions, both worth answering: what drives are in the machine (a
//! serial identifies the drive that left with the data on it), and how full the
//! filesystems are (a full disk is why the thing you are triaging stopped
//! logging).

use super::{human_bytes, read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    disks(f);
    filesystems(f);
}

fn disks(f: &mut Facts) {
    let Ok(entries) = std::fs::read_dir("/sys/block") else {
        return;
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Loopbacks, ramdisks, zram and device-mapper nodes are not drives:
        // they are files and mappings, and listing them as hardware makes an
        // inventory that cannot be checked against the machine in front of you.
        if ["loop", "ram", "zram", "dm-", "md", "sr"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let dir = entry.path();
        // `size` is in 512-byte sectors regardless of the drive's own sector
        // size — the kernel's unit, not the device's.
        let size = read_trim(dir.join("size"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|sectors| human_bytes(sectors * 512))
            .unwrap_or_default();
        let model = read_trim(dir.join("device/model")).unwrap_or_default();
        let vendor = read_trim(dir.join("device/vendor")).unwrap_or_default();
        let serial = drive_serial(&dir).unwrap_or_default();
        let rotational = matches!(read_trim(dir.join("queue/rotational")).as_deref(), Some("1"));
        let removable = matches!(read_trim(dir.join("removable")).as_deref(), Some("1"));

        let mut parts: Vec<String> = Vec::new();
        let named = format!("{vendor} {model}").trim().to_string();
        if !named.is_empty() {
            parts.push(named);
        }
        if !size.is_empty() {
            parts.push(size);
        }
        // `rotational` is 1 on every USB mass-storage device, spinning or
        // not — the bridge has no way to ask. Guessing from it would call every
        // memory stick a hard disk.
        if removable {
            parts.push("removable".into());
        } else {
            parts.push(if rotational { "HDD".into() } else { "SSD".into() });
        }
        if !serial.is_empty() {
            parts.push(format!("SN {serial}"));
        }
        rows.push((name, parts.join(", ")));
    }
    rows.sort();
    for (name, value) in rows {
        f.say("disk", &name, value);
    }
}

/// The number on the label of the drive itself.
///
/// NVMe and SATA disks publish it on the block device. A USB drive does not:
/// what is behind `sda` there is a SCSI target on a USB bridge, and the serial
/// belongs to the USB device two or three levels up the tree — which is why a
/// memory stick used to come back with no serial at all, the one case where
/// knowing which drive left the building matters most.
///
/// The climb stops after six levels and at the first `serial` there is; further
/// up are hubs and controllers, whose serials belong to the machine rather than
/// to the drive.
fn drive_serial(dir: &std::path::Path) -> Option<String> {
    if let Some(sn) = read_trim(dir.join("device/serial")) {
        return Some(sn);
    }
    let mut node = std::fs::canonicalize(dir.join("device")).ok()?;
    for _ in 0..6 {
        let Some(parent) = node.parent().map(std::path::Path::to_path_buf) else {
            break;
        };
        if let Some(sn) = read_trim(parent.join("serial")) {
            return Some(sn);
        }
        node = parent;
    }
    // `wwid` last: it is an identifier rather than a serial — for NVMe it is
    // the EUI-64, which is not the number printed on the drive.
    read_trim(dir.join("device/wwid"))
}

/// Real filesystems and how full they are.
///
/// `/proc/mounts` lists a hundred kernel and container mounts on a modern
/// machine; the ones worth an inventory are the ones backed by a device.
fn filesystems(f: &mut Facts) {
    let Some(mounts) = read_trim("/proc/mounts") else {
        return;
    };
    let mut seen: Vec<String> = Vec::new();
    for line in mounts.lines() {
        let mut cols = line.split_whitespace();
        let (Some(source), Some(point), Some(fstype)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        if !source.starts_with("/dev/") {
            continue;
        }
        if matches!(fstype, "squashfs" | "iso9660" | "devtmpfs") {
            continue;
        }
        // A `/dev/loop` mount is a snap or an image, not a disk the machine has.
        if source.starts_with("/dev/loop") {
            continue;
        }
        // The same device mounted twice (a bind, a subvolume) is one filesystem.
        let key = format!("{source} {point}");
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);

        let usage = match usage_of(point) {
            Some((total, free)) if total > 0 => {
                let used = total - free;
                format!(
                    "{} of {} used ({}%)",
                    human_bytes(used),
                    human_bytes(total),
                    used * 100 / total
                )
            }
            _ => String::new(),
        };
        let mut value = format!("{source} ({fstype})");
        if !usage.is_empty() {
            value = format!("{value} — {usage}");
        }
        f.say("filesystem", point, value);
    }
}

/// `(total, available)` bytes for the filesystem mounted at `path`.
///
/// The one thing in this module that is not a file: how much space is left is
/// not published anywhere under /proc, and `df` is a process this module would
/// otherwise have to spawn — a permission, a dependency, and an output format
/// to parse, for a number the kernel will hand over directly.
#[cfg(target_os = "linux")]
fn usage_of(path: &str) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let c = CString::new(path).ok()?;
    // SAFETY: `statvfs` writes into a struct we own and reads a NUL-terminated
    // path we just built. The call cannot fail in a way that leaves the struct
    // partly written — a non-zero return means nothing was.
    let stat = unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut stat) != 0 {
            return None;
        }
        stat
    };
    // `f_frsize` is the fragment size the block counts are in; `f_bsize` is the
    // preferred I/O size and is not the same number on every filesystem.
    let unit = if stat.f_frsize > 0 { stat.f_frsize } else { stat.f_bsize } as u64;
    Some((stat.f_blocks as u64 * unit, stat.f_bavail as u64 * unit))
}
