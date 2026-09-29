# limen-sysinfo

What **this machine** is: who made it, what it runs, what it is made of, how it
boots, and what protects it. The `fastfetch` answer with the fields a triage
question actually needs — serials, firmware, virtualization, Secure Boot, TPM,
mandatory access control.

Provides `sysinfo.local`. The local counterpart of
[`crowdstrike-sysinfo`](https://github.com/CRC-BARRACUDA/limen-crowdstrike-sysinfo),
and deliberately the same answer: the same sections, the same item names, so
"what is this host" means one thing whether you asked this machine or four
hundred of them.

## What it reports

| Section | What is in it |
|---|---|
| System | manufacturer, model, version, serial, UUID, chassis, hostname, **virtualization** |
| Operating system | distribution, version, kernel, architecture, init, uptime, boot time, **installation date**, timezone, locale, desktop, session type |
| Processor | model, vendor, physical/logical cores and sockets, max frequency, cache, microcode, VT-x/AMD-V |
| Memory | total, used (as the machine experiences it), available, swap |
| Disks | model, size, SSD/HDD, removable, **serial** — including USB drives, whose serial lives on the bridge two or three levels up the device tree |
| Filesystems | device, type, and how full each one is |
| Network | per interface: address, MAC, state, speed, MTU — plus the default route and the resolvers |
| Display | GPUs with their driver, connected outputs and their mode |
| Firmware | BIOS vendor/version/date, UEFI or legacy, board, board serial |
| Security | Secure Boot, TPM and its specification, SELinux/AppArmor and whether it is *enforcing*, kernel lockdown |

Each answer is a **fact** — `section`, `item`, `value` — not a row of a table.
This is one machine seen from several angles, and what identifies it lives
somewhere different from what it is made of.

## It spawns nothing

Everything on Linux is read from `/proc`, `/sys` and `/etc` as text. No
`dmidecode`, no `lspci`, no `lsblk`, no `df`, no `ip`, no `uname(1)`. Each of
those is a dependency that may not be installed, may be a different version, and
— on a machine being triaged — may not be the binary it claims to be.

The manifest declares one permission, `elevate`, used only when the user presses
**Scan as administrator** (below) — no `subprocess`, no `network`, no `admin`
gate. The one system call it makes is `statvfs`, for how full a filesystem is,
because that number is published nowhere under `/proc`.

## What it cannot read, it says — and can be asked to read

The DMI serials (`product_serial`, `board_serial`) are `0400 root` on most
systems. Run as an ordinary user, the module reports **which fields it could not
read and why**, above the report:

> `product_serial, board_serial` could not be read — the DMI serials are
> root-only on most systems. An absent serial here does not mean the machine has
> none.

That distinction is the whole reason the note exists. A blank row reads as a
machine without a serial number, and an inventory built on that is wrong in a
way nobody notices. The same rule applies to vendor placeholders: a board whose
serial is `To be filled by O.E.M.` has no serial, and is not recorded as having
one.

Beside the note is **Scan as administrator**, and it appears only when something
was actually unreadable — a password prompt that would recover nothing is a
password prompt for no reason. Pressing it asks the operating system, through
Limen's own elevation, to copy those four files into a folder this process
created `0700` a moment earlier; the module reads the copies, folds them into
the report, drops the note, and deletes them. `host.elevate` reports whether a
command ran and not what it printed, so a copy is the only way to read a
root-only file without this process itself being root.

Limen stays unprivileged throughout. The module declares `elevate` — and
nothing else — and never reaches for it on its own: the prompt only ever follows
a press of that button. If authorization is refused the scan already on screen
is kept, with a line saying why the fields are still missing.

## The installation date

Linux records one nowhere. There is no field to read, and `rpm -qi basesystem`
or `tune2fs -l` would both mean spawning a process this module does not spawn.

What there is instead is the **birth time** of the files an installer creates
and nothing afterwards rewrites — `/etc/machine-id`, `/etc/fstab`, `/root`,
`/var/log/installer` — and the earliest of those is the answer. A file edited
later keeps the birth time it was created with, so a rewritten `/etc/hostname`
moves its modification time and not this date.

The source is reported with it (`2026-07-04 10:35 UTC (from /root)`), because
this is derived rather than read, and an analyst putting a date in a report
needs to know which file it came from. Modification time is used for
`/etc/machine-id` alone, which is written once at first boot; every other file
here ships in a package, and its mtime is the date that package was *built* —
which on a test machine here was two months before the install.

A filesystem that keeps no birth times gets no answer rather than a guess.

## Virtualization

Answered before anything else is trusted, because it decides whether the disk in
front of you is the disk that matters. Containers first (`/.dockerenv`,
`/run/.containerenv`, PID 1's cgroup), then the hypervisor by name from DMI —
VMware, VirtualBox, QEMU/KVM, Xen, Hyper-V, Parallels, EC2, GCE, OpenStack — and
failing that, the CPU's `hypervisor` flag, which says there is one without
saying whose.

## Windows

Not collected yet, deliberately. The Windows answers live in WMI, the registry
and `GetSystemFirmwareTable` — a different shape of code, worth writing on a
Windows machine where it can be run rather than guessed at. Until then the
module returns the same envelope with no facts and a note saying why: an empty
list on its own reads as "this machine has nothing to report", which is the one
thing it must never say. `windows.rs` carries the table of which WMI class
answers which section.

## For other modules

```json
{ "method": "facts" }
```

Returns `{ os, total, facts: [{section, item, value}], notes }`. It reads the
machine when asked — unlike a fleet scan, there is nothing to schedule and
nothing to pay for.

If a `report.build` provider is installed, a **Make report** button appears.

## Languages

English and Ukrainian, screen and section headings alike.

## Tests

```sh
cargo test                              # 13 tests
cargo test show_this_machine -- --ignored --nocapture
```

The tests read the real `/proc` and `/sys` of whatever runs them, so they assert
what must be true of *any* Linux machine — a kernel, some memory, a CPU model,
no blank values, no pipes in a value, placeholders never recorded as facts. The
last one prints this machine's own report for a human to check against what they
know it to be.
