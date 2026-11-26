//! What the machine *is*: who made it, which model, and the numbers that
//! identify this one rather than another of the same kind.

use windows_sys::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW};

use super::smbios::{self, Structure};
use super::{from_wide, Facts};
use crate::facts::{chassis_name, hypervisor_guest};

pub(super) fn collect(f: &mut Facts, table: &[Structure]) {
    let system = smbios::first(table, smbios::SYSTEM);
    let field = |at: usize| system.map(|s| s.string(at)).unwrap_or_default();
    let (vendor, product) = (field(0x04), field(0x05));

    f.say("system", "Manufacturer", &vendor);
    f.say("system", "Model", &product);
    f.say("system", "Version", field(0x06));
    f.say("system", "Serial number", field(0x07));
    if let Some(uuid) = system.and_then(|s| s.bytes(0x08, 16)).and_then(smbios::uuid) {
        f.say("system", "UUID", uuid);
    }

    if let Some(chassis) = smbios::first(table, smbios::CHASSIS) {
        // The top bit of the type says whether the case has a lock, which is
        // not what kind of case it is.
        if let Some(code) = chassis.byte(0x05).map(|c| c & 0x7F).filter(|&c| c > 0) {
            f.say("system", "Chassis", chassis_name(code));
        }
        // Only when it says something the system serial did not: most vendors
        // write the same number in both.
        let serial = chassis.string(0x07);
        if serial != field(0x07) {
            f.say("system", "Chassis serial", serial);
        }
    }

    f.say("system", "Hostname", hostname());

    // Only what the firmware names outright. The processor's "a hypervisor is
    // present" flag is no evidence here: Windows runs under its own hypervisor
    // on any machine with virtualization-based security on, which is most of
    // them, and that would call every such laptop a virtual machine.
    if let Some(guest) = hypervisor_guest(&vendor, &product) {
        f.say("system", "Virtualization", guest);
    }
}

/// The name this machine answers to on the network.
fn hostname() -> String {
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    // SAFETY: `len` is the size of `buf`, in characters, as the call expects.
    let ok = unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        return std::env::var("COMPUTERNAME").unwrap_or_default();
    }
    from_wide(&buf)
}
