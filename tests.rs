//! What the module is expected to say about the machine it is running on.
//!
//! These read the real `/proc` and `/sys` of whatever runs them, so they assert
//! what must be true of *any* Linux machine — a kernel, some memory, a CPU
//! model — rather than the values of the one that happened to run the suite.

use super::*;

/// Every fact carries all three fields, and none of them is blank.
///
/// A blank value is the failure that matters here: it renders as a row saying
/// the machine has nothing of that kind, when what happened is that a file was
/// missing.
#[test]
fn every_fact_is_a_section_an_item_and_a_value() {
    let data = collect();
    let facts = data["facts"].as_array().expect("facts is a list");
    assert!(!facts.is_empty(), "no facts at all: {data}");
    for f in facts {
        for key in ["section", "item", "value"] {
            let v = f.get(key).and_then(Value::as_str).unwrap_or("");
            assert!(!v.trim().is_empty(), "empty {key} in {f}");
        }
        // The flat `section|item|value` shape the fleet module shares is only
        // unambiguous while no value contains the separator.
        assert!(
            !f["value"].as_str().unwrap().contains('|'),
            "a pipe survived into {f}"
        );
    }
    assert_eq!(data["total"].as_u64().unwrap() as usize, facts.len());
}

/// The things any machine can answer, whatever it is.
#[test]
#[cfg(target_os = "linux")]
fn the_basics_are_always_there() {
    let data = collect();
    let facts = data["facts"].as_array().unwrap();
    let has = |section: &str, item: &str| {
        facts.iter().any(|f| f["section"] == section && f["item"] == item)
    };
    for (section, item) in [
        ("system", "Hostname"),
        ("os", "Name"),
        ("os", "Kernel"),
        ("os", "Architecture"),
        ("os", "Uptime"),
        ("cpu", "Model"),
        ("cpu", "Cores"),
        ("memory", "Total"),
    ] {
        assert!(has(section, item), "{section}/{item} is missing");
    }
}

/// A value the module derives rather than copies — and the one it is easiest to
/// get wrong by a factor of 1024.
#[test]
fn sizes_read_as_sizes() {
    assert_eq!(linux::human_bytes(0), "0 B");
    assert_eq!(linux::human_bytes(1_048_576), "1 MB");
    assert_eq!(linux::human_bytes(16 * 1_073_741_824), "16.0 GB");
    // A 1 TB disk as the label on it says, near enough.
    assert_eq!(linux::human_bytes(1_000_204_886_016), "931.5 GB");
}

#[test]
fn a_duration_drops_the_units_that_are_zero() {
    assert_eq!(linux::human_duration(90), "1m");
    assert_eq!(linux::human_duration(3_600 + 120), "1h 2m");
    assert_eq!(linux::human_duration(86_400 * 12 + 3_600 * 4 + 60 * 37), "12d 4h 37m");
}

/// The epoch, and a date that is not it — the civil-from-days conversion is the
/// kind of arithmetic that is right for a year and wrong for a leap day.
#[test]
#[cfg(target_os = "linux")]
fn a_boot_time_is_a_date() {
    assert_eq!(linux::os::format_unix_utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(linux::os::format_unix_utc(1_709_164_800), "2024-02-29 00:00 UTC");
    assert_eq!(linux::os::format_unix_utc(1_735_689_599), "2024-12-31 23:59 UTC");
}

/// When the machine was installed, and where that answer came from.
///
/// Linux records no installation date, so this is derived from the birth time
/// of the files an installer creates. Two things must hold: it is a real date,
/// and it is not after the machine booted — an installation that happened after
/// the current boot is an answer read off the wrong file.
#[test]
#[cfg(target_os = "linux")]
fn the_installation_date_is_plausible_and_says_where_it_came_from() {
    let data = collect();
    let facts = data["facts"].as_array().unwrap();
    let Some(installed) = facts
        .iter()
        .find(|f| f["section"] == "os" && f["item"] == "Installed")
        .map(|f| f["value"].as_str().unwrap().to_string())
    else {
        // A filesystem with no birth times anywhere: no answer is the right
        // answer, and better than a guess.
        return;
    };

    // `2026-07-04 10:35 UTC (from /root)` — the date, and its provenance.
    assert!(installed.contains(" UTC (from /"), "no source given: {installed}");
    let date = &installed[..10];
    let year: i32 = date[..4].parse().expect("a year");
    assert!((2000..2100).contains(&year), "{installed}");

    // Installed before it was last booted, which is the one ordering that
    // cannot be violated by a machine that is running.
    if let Some(booted) = facts
        .iter()
        .find(|f| f["section"] == "os" && f["item"] == "Booted")
        .map(|f| f["value"].as_str().unwrap().to_string())
    {
        assert!(
            installed[..16] <= booted[..16],
            "installed {installed} after it booted {booted}"
        );
    }
}

/// Placeholder DMI strings are not answers.
///
/// Boards ship with "To be filled by O.E.M." in the serial field far more often
/// than anybody expects, and an inventory that records that as a serial number
/// is an inventory with a thousand identical machines in it.
#[test]
#[cfg(target_os = "linux")]
fn dmi_placeholders_are_not_facts() {
    let data = collect();
    let facts = data["facts"].as_array().unwrap();
    for f in facts {
        let v = f["value"].as_str().unwrap().to_lowercase();
        assert!(!v.contains("to be filled"), "placeholder reported as fact: {f}");
        assert!(!v.contains("default string"), "placeholder reported as fact: {f}");
    }
}

/// Every section the view knows how to title, and every note a collector can
/// raise, exists in both catalogs.
#[test]
fn the_screen_is_translated_in_both_languages() {
    for lang in ["en", "uk"] {
        for section in SECTION_ORDER.iter().chain(["security"].iter()) {
            let key = format!("section.{section}");
            assert_ne!(catalog().tr(lang, &key), key, "{lang}: {key}");
        }
        for key in [
            "ui.title", "ui.subtitle", "ui.scan", "ui.rescan", "ui.search",
            "ui.search_hint", "ui.shown_of", "ui.nothing", "col.item", "col.value",
            "note.dmi_root", "note.no_collector",
        ] {
            assert_ne!(catalog().tr(lang, key), key, "{lang}: {key} is not translated");
        }
    }
}

/// A note is written by a collector, which has no locale, and translated where
/// it is shown — with the field it is about filled in.
#[test]
fn a_note_names_the_field_it_is_about() {
    let text = note_text("note.dmi_root\u{1f}product_serial", "en");
    assert!(text.contains("product_serial"), "{text}");
    assert!(!text.contains("{}"), "the placeholder was left unfilled: {text}");
    assert!(!text.contains('\u{1f}'), "{text}");

    // A key with no entry still says something rather than rendering as a key.
    let unknown = note_text("note.not_in_the_catalog\u{1f}detail", "en");
    assert!(unknown.contains("detail"), "{unknown}");
}

/// Nothing is read until the user asks.
#[test]
fn the_screen_scans_nothing_until_it_is_asked() {
    let m = SysInfo::default();
    let idle = m.idle_view("en");
    let json = idle.to_string();
    assert!(json.contains("\"kind\":\"button\""), "{json}");
    assert!(json.contains("\"method\":\"scan\""), "{json}");
    // No table: the machine has not been read.
    assert!(!json.contains("\"kind\":\"table\""), "{json}");
}

/// Once scanned, the report is grouped into sections in reading order — what
/// identifies the machine first, not alphabetically.
#[test]
#[cfg(target_os = "linux")]
fn the_report_reads_top_down() {
    let m = SysInfo { last: collect(), scanned: true, ..Default::default() };
    let view = m.report("en", false);
    let json = view.to_string();

    let pos = |needle: &str| json.find(needle);
    let (system, os) = (pos("\"System\""), pos("\"Operating system\""));
    assert!(system.is_some() && os.is_some(), "{json}");
    assert!(system < os, "the system section comes first");
    assert!(json.contains("\"kind\":\"table\""), "the facts are in tables");
}

/// The search narrows to matching facts and says how many it left.
#[test]
#[cfg(target_os = "linux")]
fn the_search_narrows_the_report() {
    let m = SysInfo {
        last: collect(),
        scanned: true,
        query: "kernel".into(),
        ..Default::default()
    };
    let json = m.report("en", false).to_string();
    assert!(json.contains("of"), "{json}");
    assert!(json.contains("Kernel"), "the kernel row survived its own search");
    // A section with nothing matching is not drawn at all.
    assert!(!json.contains("\"Memory\""), "an empty section was drawn: {json}");
}

/// Not an assertion — prints what this machine says, for a human to check
/// against what they know it to be.
#[test]
#[ignore = "run with --ignored to read the machine"]
#[cfg(target_os = "linux")]
fn show_this_machine() {
    let data = collect();
    let mut last = String::new();
    for f in data["facts"].as_array().unwrap() {
        let section = f["section"].as_str().unwrap();
        if section != last {
            println!("\n[{section}]");
            last = section.to_string();
        }
        println!("  {:<22} {}", f["item"].as_str().unwrap(), f["value"].as_str().unwrap());
    }
    for n in data["notes"].as_array().unwrap() {
        println!("\n! {}", note_text(n.as_str().unwrap(), "en"));
    }
}

/// One sentence per reason, however many fields it cost.
///
/// Both DMI serials are unreadable for exactly the same reason, and two
/// identical paragraphs above the report is the same sentence twice.
#[test]
#[cfg(target_os = "linux")]
fn one_note_per_reason_not_per_field() {
    let mut f = linux::Facts::default();
    f.note("note.dmi_root", "product_serial");
    f.note("note.dmi_root", "board_serial");
    f.note("note.dmi_root", "product_serial"); // the same field twice is once
    let notes = f.take_notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("product_serial"), "{notes:?}");
    assert!(notes[0].contains("board_serial"), "{notes:?}");
    assert_eq!(notes[0].matches("product_serial").count(), 1, "{notes:?}");
}

/// A removable stick is not a hard disk.
///
/// `rotational` reads 1 on every USB mass-storage device, spinning or not —
/// the bridge cannot ask — so guessing from it labels every memory stick "HDD".
#[test]
#[cfg(target_os = "linux")]
fn a_removable_drive_is_not_called_a_hard_disk() {
    let data = collect();
    for f in data["facts"].as_array().unwrap() {
        if f["section"] != "disk" {
            continue;
        }
        let v = f["value"].as_str().unwrap();
        assert!(
            !(v.contains("removable") && (v.contains("HDD") || v.contains("SSD"))),
            "a removable drive was given a spindle verdict: {f}"
        );
    }
}

/// The button that asks for privileges sits under the note that explains it,
/// and only appears when something was actually unreadable — a password prompt
/// that would recover nothing is a password prompt for no reason.
#[test]
#[cfg(target_os = "linux")]
fn the_elevate_button_follows_the_note_that_explains_it() {
    let mut m = SysInfo { last: collect(), scanned: true, ..Default::default() };

    let has_note = m.last["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap_or("").starts_with("note.dmi_root"));
    let widgets = m.report("en", false);
    let widgets = widgets["widgets"].as_array().unwrap();
    let button_at = widgets.iter().position(|w| w.to_string().contains("scan_admin"));
    assert_eq!(button_at.is_some(), has_note, "the button and the note disagree");

    if let Some(at) = button_at {
        // Directly under the sentence that says why it is needed: a button
        // named "scan as administrator" means nothing until that is read.
        assert_eq!(widgets[at - 1]["kind"], "step", "{:?}", widgets[at - 1]);
        assert!(
            widgets[at - 1]["label"].as_str().unwrap().contains("root"),
            "{:?}",
            widgets[at - 1]
        );
    }

    // With the note gone — the fields were readable, or root has already been
    // asked — the button is not offered again.
    m.last["notes"] = limen_sdk_rust::json!([]);
    assert!(!m.report("en", false).to_string().contains("scan_admin"));
}

/// What an elevated read recovers is folded into the scan already on screen,
/// and the note that said the fields were unreadable goes with it.
#[test]
#[cfg(target_os = "linux")]
fn recovered_fields_replace_the_note_that_asked_for_them() {
    let dir = std::env::temp_dir().join(format!("limen-sysinfo-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("product_serial"), "PF3ZK9QW\n").unwrap();
    std::fs::write(dir.join("board_serial"), "L1HF2CN0A5X\n").unwrap();

    let mut scan = limen_sdk_rust::json!({
        "facts": [
            { "section": "system", "item": "Model", "value": "21MCCTO1WW" },
            { "section": "firmware", "item": "Board serial", "value": "old" },
        ],
        "notes": ["note.dmi_root\u{1f}product_serial, board_serial"],
        "total": 2,
    });
    let recovered = linux::merge_elevated(&mut scan, &dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(recovered, 2);
    let facts = scan["facts"].as_array().unwrap();
    // The one that was missing was added, in its own section…
    let serial = facts
        .iter()
        .find(|f| f["section"] == "system" && f["item"] == "Serial number")
        .expect("the serial was added");
    assert_eq!(serial["value"], "PF3ZK9QW");
    // …and the one that was there was replaced rather than duplicated.
    let boards: Vec<&Value> = facts.iter().filter(|f| f["item"] == "Board serial").collect();
    assert_eq!(boards.len(), 1, "{boards:?}");
    assert_eq!(boards[0]["value"], "L1HF2CN0A5X");

    // The note is gone: a value beside "could not be read" is a report nobody
    // trusts.
    assert!(scan["notes"].as_array().unwrap().is_empty(), "{}", scan["notes"]);
    assert_eq!(scan["total"], 3);
}

/// Nothing recovered leaves the scan exactly as it was, note included.
#[test]
#[cfg(target_os = "linux")]
fn an_empty_copy_changes_nothing() {
    let dir = std::env::temp_dir().join(format!("limen-sysinfo-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut scan = limen_sdk_rust::json!({
        "facts": [{ "section": "system", "item": "Model", "value": "x" }],
        "notes": ["note.dmi_root\u{1f}product_serial"],
        "total": 1,
    });
    let before = scan.clone();
    assert_eq!(linux::merge_elevated(&mut scan, &dir), 0);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(scan, before, "a failed read must not edit the report");
}

/// Every string the elevated path can show exists in both languages.
#[test]
fn the_elevated_path_is_translated() {
    for lang in ["en", "uk"] {
        for key in [
            "ui.scan_admin", "elevate.waiting", "elevate.waiting_hint", "elevate.refused",
            "elevate.unavailable", "elevate.failed", "elevate.no_tool", "elevate.no_workdir",
            "note.elevate_failed",
        ] {
            assert_ne!(catalog().tr(lang, key), key, "{lang}: {key}");
        }
    }
}

/// A drive's serial, wherever the kernel keeps it.
///
/// NVMe and SATA publish it on the block device; a USB drive does not — behind
/// `sda` is a SCSI target on a bridge, and the serial belongs to the USB device
/// two or three levels up. Read only from the block device, every memory stick
/// came back without one, which is the case where knowing which drive left the
/// building matters most.
#[test]
#[cfg(target_os = "linux")]
fn a_drive_serial_is_whole_or_absent() {
    let data = collect();
    for f in data["facts"].as_array().unwrap() {
        if f["section"] != "disk" {
            continue;
        }
        let value = f["value"].as_str().unwrap();
        let Some((_, sn)) = value.split_once("SN ") else {
            continue; // a drive that publishes none anywhere
        };
        assert!(!sn.trim().is_empty(), "an empty serial was printed: {f}");
        // Whole, not shortened: a serial cut to fit a column cannot be matched
        // against the drive it came from, which is the only thing it is for.
        assert!(!sn.contains('…') && !sn.ends_with(".."), "{f}");
    }
}

/// What this machine sends to a report provider.
///
/// The spec is the contract between the two modules: `report` knows nothing
/// about machines, and this knows nothing about documents — one hands over
/// title, subtitle, summary and sections, and gets a report of whatever kind
/// the person looking at it asks for.
#[test]
#[cfg(target_os = "linux")]
fn the_report_carries_the_whole_scan() {
    let m = SysInfo { last: collect(), scanned: true, ..Default::default() };
    let spec = m.report_spec("en").expect("a scan produces a spec");

    assert_eq!(spec["title"], catalog().tr("en", "ui.title"));
    // The subtitle is what a report is filed under: which machine this was.
    let subtitle = spec["subtitle"].as_str().unwrap();
    assert!(subtitle.contains('·'), "{subtitle}");
    assert!(subtitle.len() > 10, "the machine is not identified: {subtitle}");

    // Every section of the screen is a section of the document, in the same
    // order — a report that reorders what you just looked at is a second thing
    // to check rather than a record of the first.
    let sections = spec["sections"].as_array().unwrap();
    assert!(sections.len() >= 5, "only {} sections", sections.len());
    let on_screen: Vec<String> = {
        let facts: Vec<&Value> = m.last["facts"].as_array().unwrap().iter().collect();
        grouped(&facts).into_iter().map(|(s, _)| section_title(&s, "en")).collect()
    };
    let in_report: Vec<String> = sections
        .iter()
        .map(|s| s["heading"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(on_screen, in_report);

    // Every fact travels: the rows add up to the scan.
    let rows: usize = sections.iter().map(|s| s["rows"].as_array().unwrap().len()).sum();
    assert_eq!(rows, m.last["facts"].as_array().unwrap().len());

    // And each row is the pair the screen shows.
    let first = &sections[0]["rows"][0];
    assert_eq!(first.as_array().unwrap().len(), 2, "item and value");
}

/// What could not be read goes in the report too.
///
/// A serial missing from a document nobody can question later is worse than one
/// missing from a screen somebody is looking at.
#[test]
#[cfg(target_os = "linux")]
fn the_report_says_what_could_not_be_read() {
    let mut m = SysInfo { last: collect(), scanned: true, ..Default::default() };
    m.last["notes"] = limen_sdk_rust::json!(["note.dmi_root\u{1f}product_serial"]);
    let spec = m.report_spec("en").unwrap();
    let summary = spec["summary"].to_string();
    assert!(summary.contains("product_serial"), "{summary}");
    assert!(!summary.contains('\u{1f}'), "the note went in raw: {summary}");
}

/// Nothing scanned is not an empty report — and not the search's message
/// either, which is what it used to borrow.
#[test]
fn nothing_scanned_is_not_an_empty_report() {
    let m = SysInfo::default();
    assert!(m.report_spec("en").is_none());
    for lang in ["en", "uk"] {
        assert_ne!(catalog().tr(lang, "report.nothing"), "report.nothing", "{lang}");
        assert_ne!(
            catalog().tr(lang, "report.nothing"),
            catalog().tr(lang, "ui.nothing"),
            "{lang}: the two empty messages say the same thing"
        );
    }
}

/// The report is written in the language on screen, headings and all.
#[test]
#[cfg(target_os = "linux")]
fn the_report_follows_the_language() {
    let m = SysInfo { last: collect(), scanned: true, ..Default::default() };
    let uk = m.report_spec("uk").unwrap().to_string();
    assert!(uk.contains("Цей комп'ютер"), "the title");
    assert!(uk.contains("Операційна система"), "a section heading");
    assert!(uk.contains("Параметр"), "the column headings");
}

/// Print the spec this machine would send, for driving the report module with.
#[test]
#[ignore = "prints a spec; run with --ignored"]
#[cfg(target_os = "linux")]
fn show_report_spec() {
    let m = SysInfo { last: collect(), scanned: true, ..Default::default() };
    println!("{}", m.report_spec("en").unwrap());
}

/// The report is filed under the machine: hostname, then the serial if this
/// machine will give one up. A file called "this-machine.pdf" is one nobody can
/// find again among forty of them.
#[test]
#[cfg(target_os = "linux")]
fn the_report_is_named_after_the_machine() {
    let m = SysInfo { last: collect(), scanned: true, ..Default::default() };
    let spec = m.report_spec("en").unwrap();
    let name = spec["file_name"].as_str().expect("a name for the file");

    let host = m.last["facts"].as_array().unwrap().iter()
        .find(|f| f["section"] == "system" && f["item"] == "Hostname")
        .map(|f| f["value"].as_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(name.starts_with(&host), "{name} is not filed under {host}");

    // A serial, when this machine gives one up — and nothing invented when it
    // does not.
    let serial = m.last["facts"].as_array().unwrap().iter()
        .find(|f| f["section"] == "system" && (f["item"] == "Serial number" || f["item"] == "UUID"))
        .map(|f| f["value"].as_str().unwrap().to_string());
    match serial {
        Some(sn) => assert!(name.contains(&sn), "{name} does not carry {sn}"),
        None => assert_eq!(name, host, "{name} carries something that was never read"),
    }
    // The date is the report module's to add, not this one's.
    assert!(!name.contains("20"), "{name} dated itself");
}
