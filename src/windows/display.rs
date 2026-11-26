//! Graphics adapters, and the screens that are showing something.

use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS,
};

use super::{from_wide, Facts};

const ATTACHED_TO_DESKTOP: u32 = 0x1;
const MIRRORING_DRIVER: u32 = 0x8;

pub(super) fn collect(f: &mut Facts) {
    let mut gpus: Vec<String> = Vec::new();
    let mut screens: Vec<(String, String)> = Vec::new();

    // Windows lists an adapter once per output it has, so a card with four
    // connectors appears four times; the outputs with a screen on them are the
    // ones marked as part of the desktop.
    for device in (0..).map_while(|i| display_device(None, i)) {
        if device.StateFlags & MIRRORING_DRIVER != 0 {
            continue; // remote-desktop software, not hardware
        }
        let gpu = gpu(&from_wide(&device.DeviceString), &from_wide(&device.DeviceID));
        if !gpu.is_empty() && !gpus.contains(&gpu) {
            gpus.push(gpu);
        }
        if device.StateFlags & ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        let output = from_wide(&device.DeviceName);
        let mut value = mode(&device.DeviceName).unwrap_or_else(|| "connected".into());
        // The screen's own name, when it has one worth repeating: most answer
        // "Generic PnP Monitor", which is Windows saying it does not know.
        if let Some(monitor) = display_device(Some(&device.DeviceName), 0) {
            let name = from_wide(&monitor.DeviceString);
            if !name.is_empty() && !name.to_ascii_lowercase().contains("generic") {
                value = format!("{value}, {name}");
            }
        }
        // `\\.\DISPLAY1` — the name is the half worth reading.
        screens.push((output.trim_start_matches(r"\\.\").to_string(), value));
    }

    for (i, gpu) in gpus.iter().enumerate() {
        f.say("display", &format!("GPU {i}"), gpu);
    }
    for (output, value) in screens {
        f.say("display", &output, value);
    }
}

/// The `index`-th graphics adapter, or — given an adapter's name — the
/// `index`-th screen on it. `None` past the last.
fn display_device(adapter: Option<&[u16; 32]>, index: u32) -> Option<DISPLAY_DEVICEW> {
    // SAFETY: the structure is zeroed, says its own size as the call requires,
    // and is written only by the call; the adapter name is zero-ended, being
    // one the system wrote.
    unsafe {
        let mut device: DISPLAY_DEVICEW = std::mem::zeroed();
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        let name = adapter.map_or(std::ptr::null(), |a| a.as_ptr());
        (EnumDisplayDevicesW(name, index, &mut device, 0) != 0).then_some(device)
    }
}

/// The mode an output is running, as `2560x1440 @ 144 Hz`.
fn mode(output: &[u16; 32]) -> Option<String> {
    // SAFETY: as above — zeroed, sized, written only by the call.
    let dm = unsafe {
        let mut dm: DEVMODEW = std::mem::zeroed();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(output.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dm) == 0 {
            return None;
        }
        dm
    };
    if dm.dmPelsWidth == 0 || dm.dmPelsHeight == 0 {
        return None;
    }
    let mut mode = format!("{}x{}", dm.dmPelsWidth, dm.dmPelsHeight);
    // 0 and 1 both mean "whatever the hardware defaults to".
    if dm.dmDisplayFrequency > 1 {
        mode = format!("{mode} @ {} Hz", dm.dmDisplayFrequency);
    }
    Some(mode)
}

/// An adapter as its name and, when it is on PCI, the `vendor:device` pair that
/// identifies it whatever the driver chose to call it — the same pair the Linux
/// collector reports.
pub(crate) fn gpu(name: &str, device_id: &str) -> String {
    // `PCI\VEN_10DE&DEV_2489&SUBSYS_…`
    let hex_after = |key: &str| -> Option<String> {
        let rest = &device_id[device_id.find(key)? + key.len()..];
        let id: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
        (id.len() == 4).then(|| id.to_ascii_lowercase())
    };
    match (hex_after("VEN_"), hex_after("DEV_")) {
        (Some(vendor), Some(device)) => format!("{} [{vendor}:{device}]", name.trim()).trim().to_string(),
        _ => name.trim().to_string(),
    }
}
