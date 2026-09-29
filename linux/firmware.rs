//! The firmware underneath the OS: BIOS/UEFI, and the board it runs on.

use super::{dmi, Facts};

pub(super) fn collect(f: &mut Facts) {
    f.say("firmware", "BIOS vendor", dmi("bios_vendor").unwrap_or_default());
    f.say("firmware", "BIOS version", dmi("bios_version").unwrap_or_default());
    f.say("firmware", "BIOS date", dmi("bios_date").unwrap_or_default());
    // Whether the machine booted UEFI or legacy BIOS. The kernel exposes the
    // EFI runtime only when it was booted by one, so the directory's existence
    // is the answer — no firmware variable to read.
    f.say(
        "firmware",
        "Type",
        if std::path::Path::new("/sys/firmware/efi").exists() {
            "UEFI"
        } else {
            "Legacy BIOS"
        },
    );

    let board = format!(
        "{} {}",
        dmi("board_vendor").unwrap_or_default(),
        dmi("board_name").unwrap_or_default()
    );
    f.say("firmware", "Board", board);
    match dmi("board_serial") {
        Some(sn) => f.say("firmware", "Board serial", sn),
        None => f.note("note.dmi_root", "board_serial"),
    }
}
