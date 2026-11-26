//! Drives and the volumes on them.
//!
//! Two questions, both worth answering: what drives are in the machine (a
//! serial identifies the drive that left with the data on it), and how full the
//! volumes are (a full disk is why the thing you are triaging stopped logging).

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use super::{from_wide, wide, Facts};
use crate::facts::human_bytes;

pub(super) fn collect(f: &mut Facts) {
    disks(f);
    volumes(f);
}

/// A physical drive, opened to be asked about and not to be read.
///
/// Opened with no access rights at all, which is what lets an ordinary user do
/// it: the questions below are about the device, and none of them touches what
/// is stored on it.
struct Drive(HANDLE);

impl Drive {
    fn open(number: u32) -> Option<Drive> {
        let path = wide(&format!(r"\\.\PhysicalDrive{number}"));
        // SAFETY: the path ends with a zero; nothing else is borrowed.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        (handle != INVALID_HANDLE_VALUE && !handle.is_null()).then_some(Drive(handle))
    }

    /// Ask the drive one question; the answer is as many bytes as it gave.
    fn ask(&self, code: u32, question: &[u8], room: usize) -> Option<Vec<u8>> {
        let mut answer = vec![0u8; room];
        let mut got: u32 = 0;
        // SAFETY: both buffers are given with their own lengths, and the
        // handle is open for as long as `self` is.
        let ok = unsafe {
            DeviceIoControl(
                self.0,
                code,
                if question.is_empty() { std::ptr::null() } else { question.as_ptr().cast::<c_void>() },
                question.len() as u32,
                answer.as_mut_ptr().cast::<c_void>(),
                answer.len() as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return None;
        }
        answer.truncate(got as usize);
        Some(answer)
    }

    /// One of the drive's properties, by its number in `STORAGE_PROPERTY_ID`.
    fn property(&self, id: u32) -> Option<Vec<u8>> {
        // STORAGE_PROPERTY_QUERY: which property, "the standard query", and a
        // byte of parameters nobody uses.
        let mut query = [0u8; 12];
        query[..4].copy_from_slice(&id.to_le_bytes());
        self.ask(IOCTL_STORAGE_QUERY_PROPERTY, &query, 1024)
    }
}

impl Drop for Drive {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is closed once, here.
        unsafe { CloseHandle(self.0) };
    }
}

fn disks(f: &mut Facts) {
    // Drive numbers are handed out from zero but are not always contiguous — a
    // card reader pulled out leaves a hole — so all of a generous range is
    // tried rather than stopping at the first that is not there.
    for number in 0..64 {
        let Some(drive) = Drive::open(number) else {
            continue;
        };
        let Some(device) = drive.property(0).map(|raw| describe(&raw)) else {
            continue;
        };
        // DISK_GEOMETRY_EX: the geometry nobody needs, then the size at 24.
        let size = drive
            .ask(IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], 256)
            .and_then(|raw| Some(u64::from_le_bytes(raw.get(24..32)?.try_into().ok()?)))
            .filter(|&bytes| bytes > 0);
        // DEVICE_SEEK_PENALTY_DESCRIPTOR: whether moving to another place on
        // the drive costs time, which is what a spinning disk is.
        let seeks = drive.property(7).and_then(|raw| raw.get(8).map(|&b| b != 0));

        let mut parts: Vec<String> = Vec::new();
        if !device.name.is_empty() {
            parts.push(device.name.clone());
        }
        if let Some(bytes) = size {
            parts.push(human_bytes(bytes));
        }
        parts.extend(kind(&device, seeks).map(str::to_string));
        if !device.bus.is_empty() {
            parts.push(device.bus.to_string());
        }
        if !device.serial.is_empty() {
            parts.push(format!("SN {}", device.serial));
        }
        f.say("disk", &format!("Disk {number}"), parts.join(", "));
    }
}

/// What a drive says about itself.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Device {
    pub name: String,
    pub serial: String,
    pub bus: &'static str,
    pub removable: bool,
}

/// Read a `STORAGE_DEVICE_DESCRIPTOR`: a fixed head, in which each string is
/// given as the offset it starts at — zero when the drive has none.
pub(crate) fn describe(raw: &[u8]) -> Device {
    let u32_at = |at: usize| {
        raw.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
    };
    let text_at = |at: usize| -> String {
        let at = u32_at(at) as usize;
        if at == 0 || at >= raw.len() {
            return String::new();
        }
        let end = raw[at..].iter().position(|&b| b == 0).map_or(raw.len(), |e| at + e);
        String::from_utf8_lossy(&raw[at..end]).trim().to_string()
    };
    let (vendor, product) = (text_at(12), text_at(16));
    Device {
        name: format!("{vendor} {product}").trim().to_string(),
        serial: text_at(24),
        bus: match u32_at(28) {
            1 => "SCSI",
            3 => "ATA",
            7 => "USB",
            8 => "RAID",
            10 => "SAS",
            11 => "SATA",
            12 => "SD",
            13 => "MMC",
            14 | 15 => "virtual",
            16 => "Storage Spaces",
            17 => "NVMe",
            _ => "",
        },
        removable: raw.get(10).is_some_and(|&b| b != 0),
    }
}

/// Removable, solid-state or spinning — or nothing, when the drive will not
/// say.
///
/// A USB bridge has no way to ask what is behind it and answers "it seeks" for
/// every memory stick, so anything on USB is called removable rather than
/// guessed at; and nothing on NVMe spins.
pub(crate) fn kind(device: &Device, seeks: Option<bool>) -> Option<&'static str> {
    if device.removable || device.bus == "USB" {
        return Some("removable");
    }
    if device.bus == "NVMe" {
        return Some("SSD");
    }
    seeks.map(|s| if s { "HDD" } else { "SSD" })
}

/// The lettered volumes on drives the machine has.
fn volumes(f: &mut Facts) {
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    // SAFETY: takes nothing; one bit per letter.
    let letters = unsafe { GetLogicalDrives() };
    for i in 0..26u8 {
        if letters & (1 << i) == 0 {
            continue;
        }
        let letter = char::from(b'A' + i);
        let root = wide(&format!(r"{letter}:\"));
        // Network shares and optical drives are not disks this machine has.
        // SAFETY: `root` ends with a zero.
        let drive_type = unsafe { GetDriveTypeW(root.as_ptr()) };
        if drive_type != DRIVE_FIXED && drive_type != DRIVE_REMOVABLE {
            continue;
        }

        let (mut label, mut fs) = ([0u16; 261], [0u16; 261]);
        let (mut total, mut free) = (0u64, 0u64);
        // SAFETY: every buffer is given with its own length; the outputs not
        // wanted are null, which the calls allow.
        let known = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            ) != 0
                && GetDiskFreeSpaceExW(root.as_ptr(), std::ptr::null_mut(), &mut total, &mut free) != 0
        };
        // A card reader with no card in it has a letter and nothing behind it.
        if !known || total == 0 {
            continue;
        }

        let (label, fs) = (from_wide(&label), from_wide(&fs));
        let used = total.saturating_sub(free);
        let what = if label.trim().is_empty() { fs } else { format!("{} ({fs})", label.trim()) };
        f.say(
            "filesystem",
            &format!("{letter}:"),
            format!(
                "{what} — {} of {} used ({}%)",
                human_bytes(used),
                human_bytes(total),
                used.saturating_mul(100) / total
            ),
        );
    }
}
