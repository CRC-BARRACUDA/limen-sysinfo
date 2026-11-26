//! The firmware underneath the OS: BIOS/UEFI, and the board it runs on.

use windows_sys::Win32::System::SystemInformation::GetFirmwareType;

use super::smbios::{self, Structure};
use super::Facts;

pub(super) fn collect(f: &mut Facts, table: &[Structure]) {
    if let Some(bios) = smbios::first(table, smbios::BIOS) {
        f.say("firmware", "BIOS vendor", bios.string(0x04));
        f.say("firmware", "BIOS version", bios.string(0x05));
        f.say("firmware", "BIOS date", bios.string(0x08));
    }

    // Whether the machine booted UEFI or legacy BIOS, as Windows itself was
    // told when it started.
    let mut kind: i32 = 0;
    // SAFETY: writes the one number it is given room for.
    if unsafe { GetFirmwareType(&mut kind) } != 0 {
        f.say(
            "firmware",
            "Type",
            match kind {
                1 => "Legacy BIOS",
                2 => "UEFI",
                _ => "",
            },
        );
    }

    if let Some(board) = smbios::first(table, smbios::BOARD) {
        f.say("firmware", "Board", format!("{} {}", board.string(0x04), board.string(0x05)));
        f.say("firmware", "Board serial", board.string(0x07));
    }
}
