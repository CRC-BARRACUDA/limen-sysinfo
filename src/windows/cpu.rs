//! The processor: what it calls itself in the registry, and how the system
//! says its cores and caches are laid out.

use windows_sys::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, RelationAll,
};

use super::{reg_bytes, reg_string, reg_u32, Facts};

const CPU0: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

pub(super) fn collect(f: &mut Facts) {
    f.say("cpu", "Model", reg_string(CPU0, "ProcessorNameString").unwrap_or_default());
    f.say("cpu", "Vendor", reg_string(CPU0, "VendorIdentifier").unwrap_or_default());

    let layout = layout(&processor_information());
    if layout.logical > 0 {
        f.say(
            "cpu",
            "Cores",
            match (layout.cores, layout.sockets) {
                (0, _) => format!("{} logical", layout.logical),
                (c, s) if s > 1 => format!("{c} physical, {} logical, {s} sockets", layout.logical),
                (c, _) => format!("{c} physical, {} logical", layout.logical),
            },
        );
    }

    // The speed on the box, not the fastest a core may go: Windows records the
    // nominal clock and nothing about turbo, so it is called what it is rather
    // than given the name of the Linux figure it is not.
    if let Some(mhz) = reg_u32(CPU0, "~MHz").filter(|&m| m > 0) {
        f.say("cpu", "Base frequency", format!("{:.2} GHz", f64::from(mhz) / 1000.0));
    }
    f.say("cpu", "Cache", layout.caches.join(" · "));
    f.say("cpu", "Microcode", microcode());
    f.say("cpu", "Virtualization", virtualization());
}

/// The microcode revision the processor is running.
///
/// Eight bytes in the registry, as the processor's own register has them: the
/// revision is the upper half, the lower is unused.
fn microcode() -> String {
    reg_bytes(CPU0, "Update Revision")
        .and_then(|raw| Some(u32::from_le_bytes(raw.get(4..8)?.try_into().ok()?)))
        .filter(|&rev| rev != 0)
        .map(|rev| format!("{rev:#x}"))
        .unwrap_or_default()
}

/// How the processor is laid out, counted from the system's own records.
#[derive(Default, Debug, PartialEq)]
pub(crate) struct Layout {
    pub cores: usize,
    pub logical: usize,
    pub sockets: usize,
    /// `L1d 48K`, `L2 2048K`… as the first processor has them.
    pub caches: Vec<String>,
}

/// Everything `GetLogicalProcessorInformationEx` has to say, as it gives it.
fn processor_information() -> Vec<u8> {
    let mut len: u32 = 0;
    // SAFETY: with no buffer the call only reports the size; with one it
    // writes at most `len` bytes. The buffer is of `u64` so that the records
    // the system writes into it sit where their alignment wants them.
    unsafe {
        GetLogicalProcessorInformationEx(RelationAll, std::ptr::null_mut(), &mut len);
        if len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        if GetLogicalProcessorInformationEx(RelationAll, buf.as_mut_ptr().cast(), &mut len) == 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len as usize).to_vec()
    }
}

/// Read those records.
///
/// Each starts with what it describes and how long it is, which is what makes
/// them walkable; what follows depends on the kind. They are read by offset
/// rather than through the system's structures because the records are of
/// different lengths and a core's can carry more than one group of processors.
pub(crate) fn layout(records: &[u8]) -> Layout {
    const CORE: u32 = 0;
    const CACHE: u32 = 2;
    const PACKAGE: u32 = 3;

    let u16_at = |at: usize| records.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| {
        records.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let u64_at = |at: usize| {
        records.get(at..at + 8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
    };

    let mut out = Layout::default();
    let mut caches: Vec<(String, u32)> = Vec::new();
    let mut at = 0;
    while let (Some(kind), Some(size)) = (u32_at(at), u32_at(at + 4)) {
        let size = size as usize;
        if size < 8 || at + size > records.len() {
            break;
        }
        match kind {
            CORE => {
                out.cores += 1;
                // The processors of this core, one bit each, in as many groups
                // as it spans (more than one only beyond 64 processors).
                let groups = u16_at(at + 30).unwrap_or(0) as usize;
                for g in 0..groups {
                    let mask = at + 32 + g * 16;
                    if mask + 8 <= at + size {
                        out.logical += u64_at(mask).unwrap_or(0).count_ones() as usize;
                    }
                }
            }
            PACKAGE => out.sockets += 1,
            CACHE => {
                // The caches of the first processor, as Linux reports cpu0's:
                // on a chip with two kinds of core the others differ, and a
                // list of every size on the chip answers nobody's question.
                let first_cpu = u64_at(at + 40).is_some_and(|m| m & 1 == 1)
                    && u16_at(at + 48) == Some(0);
                if let (true, Some(&level), Some(bytes), Some(kind)) =
                    (first_cpu, records.get(at + 8), u32_at(at + 12), u32_at(at + 16))
                {
                    let name = match kind {
                        2 => format!("L{level}d"),
                        1 => format!("L{level}i"),
                        _ => format!("L{level}"),
                    };
                    if !caches.iter().any(|(n, _)| *n == name) {
                        caches.push((name, bytes));
                    }
                }
            }
            _ => {}
        }
        at += size;
    }
    caches.sort();
    out.caches = caches.into_iter().map(|(name, bytes)| format!("{name} {}K", bytes / 1024)).collect();
    out
}

/// Whether this processor can host virtual machines.
///
/// The processor's own flag, as on Linux — with one Windows complication: once
/// Windows is running under its own hypervisor (Hyper-V, or just
/// virtualization-based security) that hypervisor hides the flag from
/// everything above it. A machine in that state is using the feature, so it is
/// said rather than reported as a processor without it.
#[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
fn virtualization() -> String {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::__cpuid;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::__cpuid;

    // `__cpuid` is safe on the toolchains that know every x86 has it, and
    // unsafe on the older ones.
    #[allow(unused_unsafe)]
    let (basic, extended_max) = unsafe { (__cpuid(1), __cpuid(0x8000_0000).eax) };
    let vmx = basic.ecx & (1 << 5) != 0;
    #[allow(unused_unsafe)]
    let svm = extended_max >= 0x8000_0001 && unsafe { __cpuid(0x8000_0001) }.ecx & (1 << 2) != 0;
    let hypervisor = basic.ecx & (1 << 31) != 0;

    if vmx {
        "Intel VT-x".into()
    } else if svm {
        "AMD-V".into()
    } else if hypervisor {
        "in use by a running hypervisor".into()
    } else {
        String::new()
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "x86")))]
fn virtualization() -> String {
    String::new()
}
