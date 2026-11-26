//! What the machine *is*: who made it, which model, and the numbers that
//! identify this one rather than another of the same kind.

use super::{dmi, read_trim, Facts};

pub(super) fn collect(f: &mut Facts) {
    f.say("system", "Manufacturer", dmi("sys_vendor").unwrap_or_default());
    f.say("system", "Model", dmi("product_name").unwrap_or_default());
    f.say("system", "Version", dmi("product_version").unwrap_or_default());

    // The serials are `0400 root`. Read as an ordinary user they come back
    // empty, which reads exactly like a machine that has no serial — so the
    // difference is said out loud instead.
    match dmi("product_serial") {
        Some(sn) => f.say("system", "Serial number", sn),
        None => f.note("note.dmi_root", "product_serial"),
    }
    f.say("system", "UUID", dmi("product_uuid").unwrap_or_default());
    f.say("system", "Chassis", chassis());
    f.say("system", "Hostname", read_trim("/proc/sys/kernel/hostname").unwrap_or_default());
    if let Some(virt) = virtualization() {
        f.say("system", "Virtualization", virt);
    }
}

/// The DMI chassis type, as a word rather than the number SMBIOS stores.
///
/// Worth having in an inventory: "this serial is a laptop" answers a different
/// question from "this serial is a rack server", and the two are handled by
/// different people.
fn chassis() -> String {
    let Some(code) = read_trim("/sys/class/dmi/id/chassis_type") else {
        return String::new();
    };
    match code.trim() {
        "1" => "Other",
        "2" => "Unknown",
        "3" | "4" | "6" | "7" | "15" => "Desktop",
        "5" => "Pizza box",
        "8" | "9" | "10" | "14" => "Laptop",
        "11" => "Hand held",
        "12" => "Docking station",
        "13" => "All in one",
        "16" => "Lunch box",
        "17" | "23" | "28" => "Server",
        "18" => "Expansion chassis",
        "21" => "Peripheral chassis",
        "22" => "RAID chassis",
        "24" => "Sealed-case PC",
        "30" => "Tablet",
        "31" => "Convertible",
        "32" => "Detachable",
        other => return format!("Type {other}"),
    }
    .to_string()
}

/// Whether this is a virtual machine or a container, and which kind.
///
/// The first question asked of any host under triage, and the one that decides
/// whether the disk in front of you is the disk that matters. Read from the
/// same places `systemd-detect-virt` reads, without being `systemd-detect-virt`
/// — which is not installed everywhere and is a process this module does not
/// need to spawn.
fn virtualization() -> Option<String> {
    // A container first: it is also running on some hypervisor, and the
    // container is the more immediate answer.
    if std::path::Path::new("/.dockerenv").exists() {
        return Some("Docker container".into());
    }
    if std::path::Path::new("/run/.containerenv").exists() {
        return Some("Podman container".into());
    }
    if let Some(cg) = read_trim("/proc/1/cgroup") {
        for (marker, name) in [("/docker", "Docker"), ("/lxc", "LXC"), ("/kubepods", "Kubernetes")] {
            if cg.contains(marker) {
                return Some(format!("{name} container"));
            }
        }
    }
    if read_trim("/proc/1/environ").is_some_and(|e| e.contains("container=lxc")) {
        return Some("LXC container".into());
    }

    // Then the hypervisor, which DMI usually names outright.
    let vendor = dmi("sys_vendor").unwrap_or_default().to_ascii_lowercase();
    let product = dmi("product_name").unwrap_or_default().to_ascii_lowercase();
    let both = format!("{vendor} {product}");
    for (marker, name) in [
        ("vmware", "VMware"),
        ("virtualbox", "VirtualBox"),
        ("innotek", "VirtualBox"),
        ("qemu", "QEMU/KVM"),
        ("kvm", "QEMU/KVM"),
        ("bochs", "QEMU"),
        ("xen", "Xen"),
        ("microsoft corporation virtual", "Hyper-V"),
        ("parallels", "Parallels"),
        ("amazon ec2", "Amazon EC2"),
        ("google", "Google Compute Engine"),
        ("alibaba", "Alibaba Cloud"),
        ("openstack", "OpenStack"),
    ] {
        if both.contains(marker) {
            return Some(format!("{name} guest"));
        }
    }
    if read_trim("/sys/hypervisor/type").is_some() {
        return Some("Xen guest".into());
    }
    // Last resort: the CPU says it is under a hypervisor but will not say whose.
    if read_trim("/proc/cpuinfo")
        .is_some_and(|c| c.lines().any(|l| l.starts_with("flags") && l.contains(" hypervisor")))
    {
        return Some("Virtual machine (hypervisor present)".into());
    }
    None
}
