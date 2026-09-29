//! `sysinfo` — a native Limen module answering "what is this machine?".
//!
//! Provides `sysinfo.local`. One screen: press **Scan**, and the machine
//! describes itself — who made it, what it is, what it runs, what it is made
//! of, how it boots and what protects it.
//!
//! Every answer is a **fact**: `section`, `item`, `value`. Not a table of like
//! things, because this is one machine seen from several angles, and what
//! identifies it (a serial) lives somewhere different from what it is made of
//! (a disk). The sections and the item names are the same ones
//! `crowdstrike-sysinfo` reports for a fleet host, so the local answer and the
//! fleet answer agree rather than being two dialects of the same inventory.
//!
//! On Linux everything is read from `/proc`, `/sys` and `/etc` — no
//! `dmidecode`, no `lspci`, no `df`, no subprocess at all. Each of those is a
//! dependency that may be missing, may be a different version, and, on a
//! machine being triaged, may not be the binary it claims to be. The module
//! declares **no permissions**.
//!
//! Windows is a stub that says so; see [`windows`].

use limen_sdk_rust::ui::{button, label, row, separator, step, table, text, window};
use limen_sdk_rust::{export_module, rpc, Catalog, Handler, Host, RpcError, Value};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::collect;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::collect;

/// Anywhere else: the same envelope, and the reason it is empty.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn collect() -> Value {
    limen_sdk_rust::json!({
        "os": std::env::consts::OS,
        "note": "No collector for this platform.",
        "total": 0,
        "facts": [],
        "notes": [format!("note.no_collector\u{1f}{}", std::env::consts::OS)],
    })
}

/// The module's strings. English is the fallback; a key missing from a catalog
/// renders as itself, which is loud enough to be noticed on screen.
fn catalog() -> &'static Catalog {
    static C: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        Catalog::new(&[
            ("en", include_str!("locales/en.toml")),
            ("uk", include_str!("locales/uk.toml")),
        ])
    })
}

/// The order sections are shown in: what somebody identifying a machine reads
/// first, not alphabetical. A section a collector adds that is not listed here
/// still appears, after these.
const SECTION_ORDER: [&str; 9] = [
    "system",
    "os",
    "cpu",
    "memory",
    "disk",
    "filesystem",
    "network",
    "display",
    "firmware",
];

#[derive(Default)]
struct SysInfo {
    /// Whether the user has scanned this session. The screen enumerates nothing
    /// until asked — reopening the tab shows what was found, not a fresh read.
    scanned: bool,
    /// The last scan, kept so the view can redraw without reading the machine
    /// again.
    last: Value,
    /// The filter text, restored into the search box on redraw.
    query: String,
    /// An elevated read in flight: the host's id for it, and the private
    /// directory it is copying into.
    #[cfg(target_os = "linux")]
    elevation: Option<(u64, std::path::PathBuf)>,
}

impl Handler for SysInfo {
    fn capabilities(&self) -> Vec<String> {
        vec!["sysinfo.local".into()]
    }

    fn invoke(
        &mut self,
        _capability: &str,
        method: &str,
        params: Value,
        host: &Host,
    ) -> Result<Value, RpcError> {
        let lang = host.locale();
        match method {
            // The capability method: the facts as data, for another module.
            // Reads the machine when asked, because reading it is cheap and
            // local — unlike a fleet scan, there is nothing to schedule.
            "facts" => Ok(collect()),
            "ui" => Ok(if self.scanned {
                self.report(&lang, Self::has_reports(host))
            } else {
                self.idle_view(&lang)
            }),
            // Hand the last scan to whatever report provider is installed.
            "make_report" => Ok(self.make_report(host, &lang)),
            "scan" => {
                self.query = params
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.last = collect();
                self.scanned = true;
                Ok(self.report(&lang, Self::has_reports(host)))
            }
            #[cfg(target_os = "linux")]
            "scan_admin" => Ok(self.scan_admin(host, &lang)),
            #[cfg(target_os = "linux")]
            "admin_poll" => Ok(self.admin_poll(host, &lang)),
            other => Err(RpcError::new(
                rpc::METHOD_NOT_FOUND,
                format!("No method {other}"),
            )),
        }
    }
}

impl SysInfo {
    /// Nothing is read until asked: a line about what this does, and a button.
    fn idle_view(&self, lang: &str) -> Value {
        let t = |k: &str| catalog().tr(lang, k);
        window(
            t("ui.title"),
            vec![
                label(t("ui.title")).heading(),
                label(t("ui.subtitle")).weak(),
                separator(),
                button(t("ui.scan"), "sysinfo.local", "scan").primary(),
            ],
        )
    }

    /// Ask the operating system for the fields an ordinary user cannot read.
    ///
    /// Root cannot hand anything back through `host.elevate` — it reports
    /// whether the command ran, not what it printed — so what is elevated is a
    /// **copy** into a directory this process already owns. The directory is
    /// made `0700` *before* the copy, so the serials are never readable by
    /// anyone else on the machine even for the moment they exist on disk.
    #[cfg(target_os = "linux")]
    fn scan_admin(&mut self, host: &Host, lang: &str) -> Value {
        use std::os::unix::fs::PermissionsExt;
        let t = |k: &str| catalog().tr(lang, k);

        let paths = linux::root_only_paths();
        if paths.is_empty() {
            return self.report(lang, Self::has_reports(host));
        }
        let dir = std::env::temp_dir().join(format!(
            "limen-sysinfo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        // `create_dir`, not `create_dir_all`: it fails if the path is already
        // there, which is what stops anything else claiming the name first.
        if std::fs::create_dir(&dir).is_err()
            || std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).is_err()
        {
            return self.elevation_failed(lang, t("elevate.no_workdir"));
        }

        // `install -m 0644`, because a plain `cp` would carry the source's
        // `0400 root` across and leave this process unable to read its own
        // copy. The destination is private either way.
        let Some(copier) = ["/usr/bin/install", "/bin/install"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
        else {
            let _ = std::fs::remove_dir_all(&dir);
            return self.elevation_failed(lang, t("elevate.no_tool"));
        };
        let dest = dir.to_string_lossy().into_owned();
        let mut argv: Vec<&str> = vec![copier, "-m", "0644"];
        argv.extend(paths.iter().map(String::as_str));
        argv.push(&dest);

        match host.elevate_async(&argv, None) {
            Some(id) => {
                self.elevation = Some((id, dir));
                host.log("[sysinfo] asking for the DMI serials as root");
                self.waiting_view(lang)
            }
            None => {
                let _ = std::fs::remove_dir_all(&dir);
                self.elevation_failed(lang, t("elevate.refused"))
            }
        }
    }

    /// One tick of the elevated read: still asking, still running, or done.
    #[cfg(target_os = "linux")]
    fn admin_poll(&mut self, host: &Host, lang: &str) -> Value {
        let t = |k: &str| catalog().tr(lang, k);
        let Some((id, dir)) = self.elevation.clone() else {
            return self.report(lang, Self::has_reports(host));
        };
        match host.elevate_state(id) {
            // The prompt is on screen, or the copy is running. One frame each,
            // and the view asks to be called again.
            limen_sdk_rust::ElevateState::Authorizing
            | limen_sdk_rust::ElevateState::Running => self.waiting_view(lang),
            limen_sdk_rust::ElevateState::Done(result) => {
                self.elevation = None;
                let recovered = if result.ok() {
                    linux::merge_elevated(&mut self.last, &dir)
                } else {
                    0
                };
                // The copies are deleted whatever happened: they are serial
                // numbers on disk, and nothing needs them after this.
                let _ = std::fs::remove_dir_all(&dir);
                if recovered == 0 {
                    let why = match result.reason.as_str() {
                        "refused" => t("elevate.refused"),
                        "unavailable" => t("elevate.unavailable"),
                        _ => t("elevate.failed"),
                    };
                    host.log(&format!("[sysinfo] elevated read: {}", result.message));
                    return self.elevation_failed(lang, why);
                }
                host.log(&format!("[sysinfo] recovered {recovered} root-only field(s)"));
                self.report(lang, Self::has_reports(host))
            }
        }
    }

    /// While the operating system is asking. The view asks to be polled, which
    /// is what moves the spinner and what notices the answer.
    #[cfg(target_os = "linux")]
    fn waiting_view(&self, lang: &str) -> Value {
        let t = |k: &str| catalog().tr(lang, k);
        let mut view = window(
            t("ui.title"),
            vec![
                label(t("ui.title")).heading(),
                step(t("elevate.waiting"), "loading"),
                label(t("elevate.waiting_hint")).weak(),
            ],
        );
        view["auto"] = limen_sdk_rust::json!({
            "capability": "sysinfo.local",
            "method": "admin_poll",
            "args": {},
        });
        view
    }

    /// The report, with a line saying the elevated read did not happen — the
    /// scan that was already on screen is still good, and losing it because the
    /// password prompt was dismissed would be the wrong answer.
    #[cfg(target_os = "linux")]
    fn elevation_failed(&mut self, lang: &str, why: String) -> Value {
        if let Some(notes) = self.last.get_mut("notes").and_then(|n| n.as_array_mut()) {
            let line = Value::String(format!("note.elevate_failed\u{1f}{why}"));
            if !notes.contains(&line) {
                notes.push(line);
            }
        }
        self.report(lang, false)
    }

    /// Whether a report provider is installed right now.
    ///
    /// Asked each draw rather than cached: `report.build` is optional, and a
    /// provider installed while this tab is open should make the button appear.
    fn has_reports(host: &Host) -> bool {
        host.capabilities().iter().any(|c| c == "report.build")
    }

    /// The last scan as a report: a summary and one table per section.
    fn make_report(&self, host: &Host, lang: &str) -> Value {
        let t = |k: &str| catalog().tr(lang, k);
        let Some(spec) = self.report_spec(lang) else {
            // Nothing has been read yet — which is not the same as a scan that
            // found nothing, and says so in its own words rather than borrowing
            // the search's.
            return window(t("ui.title"), vec![label(t("report.nothing")).strong()]);
        };
        match host.call("report.build", "build", spec) {
            // A provider that renders answers with a view — its preview of the
            // document, with the buttons that write the file.
            Ok(v) if v.get("widgets").is_some() => v,
            // One that exports straight to a file writes it, opens it, and
            // acknowledges with nothing.
            Ok(_) => window(
                t("ui.title"),
                vec![label(t("report.exported")).strong(), label(t("report.exported_hint")).weak()],
            ),
            Err(e) => window(
                t("ui.title"),
                vec![label(t("report.failed")).strong(), label(format!("{e}")).weak()],
            ),
        }
    }

    /// The last scan as a report spec, or `None` if there is nothing to report.
    ///
    /// Separate from the call that sends it so it can be read in a test: what
    /// goes into a report is the part worth pinning, and the sending is one
    /// line of plumbing.
    ///
    /// Every section of the scan becomes a section of the document, in the
    /// order the screen shows them — a report that reorders what you just
    /// looked at is a second thing to check rather than a record of the first.
    fn report_spec(&self, lang: &str) -> Option<Value> {
        let t = |k: &str| catalog().tr(lang, k);
        let facts: Vec<&Value> = self
            .last
            .get("facts")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        if facts.is_empty() {
            return None;
        }
        let sections: Vec<Value> = grouped(&facts)
            .into_iter()
            .map(|(section, rows)| {
                limen_sdk_rust::json!({
                    "heading": section_title(&section, lang),
                    "columns": [t("col.item"), t("col.value")],
                    "rows": rows.iter()
                        .map(|f| vec![cell(f, "item"), cell(f, "value")])
                        .collect::<Vec<Vec<String>>>(),
                })
            })
            .collect();
        let find = |section: &str, item: &str| {
            facts
                .iter()
                .find(|f| cell(f, "section") == section && cell(f, "item") == item)
                .map(|f| cell(f, "value"))
                .unwrap_or_default()
        };
        // What could not be read travels with the report. A serial missing
        // from a document nobody can question later is worse than one missing
        // from a screen somebody is looking at.
        let mut summary = vec![
            format!("{}: {}", t("section.system"), find("system", "Manufacturer")),
            format!("{}: {}", t("section.cpu"), find("cpu", "Model")),
            format!("{}: {}", t("section.memory"), find("memory", "Total")),
        ];
        for note in self.last.get("notes").and_then(Value::as_array).into_iter().flatten() {
            if let Some(raw) = note.as_str() {
                summary.push(note_text(raw, lang));
            }
        }
        // What the file is filed under: the machine, the one serial that
        // identifies this one rather than another of the same model, and — added
        // by the report module — the date. A report called "this-machine.pdf"
        // is one nobody can find again among forty of them.
        let mut stem = find("system", "Hostname");
        if stem.is_empty() {
            stem = t("ui.title");
        }
        for item in ["Serial number", "UUID"] {
            let serial = find("system", item);
            if !serial.is_empty() {
                stem = format!("{stem}_{serial}");
                break;
            }
        }

        Some(limen_sdk_rust::json!({
            "title": t("ui.title"),
            "file_name": stem,
            // The three lines that identify the machine, in the subtitle where
            // a report is filed under them.
            "subtitle": format!("{} · {} · {}",
                find("system", "Hostname"), find("system", "Model"), find("os", "Name")),
            // A view: the provider answers with its preview of the document,
            // and the choice of paper or screen belongs to whoever is looking
            // at it rather than to this module.
            "format": "view",
            "summary": summary,
            "charts": [],
            "sections": sections,
        }))
    }

    /// The machine, section by section.
    /// `reports` says whether a provider is installed, and so whether the
    /// toolbar carries a button that would otherwise do nothing.
    fn report(&self, lang: &str, reports: bool) -> Value {
        let t = |k: &str| catalog().tr(lang, k);
        let facts = self
            .last
            .get("facts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let needle = self.query.trim().to_lowercase();
        let matches = |f: &Value| -> bool {
            if needle.is_empty() {
                return true;
            }
            let hay = format!(
                "{} {} {}",
                cell(f, "section"),
                cell(f, "item"),
                cell(f, "value")
            )
            .to_lowercase();
            hay.contains(&needle)
        };

        let shown: Vec<&Value> = facts.iter().filter(|f| matches(f)).collect();
        let mut toolbar = vec![button(t("ui.rescan"), "sysinfo.local", "scan").primary()];
        if reports {
            // Not `open_in_tab`: the provider answers with a pop-up, and a
            // pop-up opened into a tab of its own leaves that tab empty behind
            // it — nothing to show, and nothing to close.
            toolbar.push(button(t("ui.report"), "sysinfo.local", "make_report"));
        }

        // The search sits on its own line under the buttons: sharing a row with
        // them leaves it a quarter of the width, and a search box the size of a
        // button is one you cannot read what you typed into.
        let mut widgets = vec![
            label(t("ui.title")).heading(),
            row(toolbar),
            text("query")
                .label(t("ui.search"))
                .placeholder(t("ui.search_hint"))
                .default(&self.query),
        ];
        if !needle.is_empty() {
            widgets.push(
                label(
                    t("ui.shown_of")
                        .replace("{shown}", &shown.len().to_string())
                        .replace("{total}", &facts.len().to_string()),
                )
                .weak(),
            );
        }

        // What could not be read, before the facts rather than after them: a
        // serial that is missing because this process may not read it is not
        // the same as a machine without one, and the difference belongs where
        // it will be seen.
        for note in self.last.get("notes").and_then(Value::as_array).into_iter().flatten() {
            let Some(raw) = note.as_str() else { continue };
            widgets.push(step(note_text(raw, lang), "pending"));
            // The offer to ask for privileges goes directly under the sentence
            // that says why it is needed, rather than in the toolbar: a button
            // named "scan as administrator" means nothing until you have read
            // the line above it, and everything once you have.
            #[cfg(target_os = "linux")]
            if raw.starts_with("note.dmi_root") {
                widgets.push(button(t("ui.scan_admin"), "sysinfo.local", "scan_admin"));
            }
        }

        let sections = grouped(&shown);
        if sections.is_empty() {
            widgets.push(label(t("ui.nothing")).weak());
        }
        for (section, rows) in sections {
            widgets.push(separator());
            widgets.push(label(section_title(&section, lang)).strong());
            widgets.push(table(
                vec![t("col.item"), t("col.value")],
                rows.iter()
                    .map(|f| vec![cell(f, "item"), cell(f, "value")])
                    .collect::<Vec<Vec<String>>>(),
            ));
        }
        window(t("ui.title"), widgets)
    }
}

/// A fact's field as a string, or "" — the view never has to unwrap.
fn cell(f: &Value, key: &str) -> String {
    f.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Facts grouped by section, in reading order, with anything unlisted after.
fn grouped(facts: &[&Value]) -> Vec<(String, Vec<Value>)> {
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    let mut push = |section: &str, facts: &[&Value]| {
        let rows: Vec<Value> = facts
            .iter()
            .filter(|f| cell(f, "section") == section)
            .map(|f| (*f).clone())
            .collect();
        if !rows.is_empty() {
            out.push((section.to_string(), rows));
        }
    };
    for section in SECTION_ORDER {
        push(section, facts);
    }
    let mut extras: Vec<String> = facts
        .iter()
        .map(|f| cell(f, "section"))
        .filter(|s| !SECTION_ORDER.contains(&s.as_str()))
        .collect();
    extras.sort();
    extras.dedup();
    for section in extras {
        push(&section, facts);
    }
    out
}

/// A section heading, translated. A section a collector adds that has no
/// heading of its own falls back to its own name.
fn section_title(section: &str, lang: &str) -> String {
    let key = format!("section.{section}");
    let shown = catalog().tr(lang, &key);
    if shown == key {
        section.to_string()
    } else {
        shown
    }
}

/// A note, translated. Collectors raise `key` or `key\u{1f}argument`, because
/// they have no locale — the same contract the other modules use for a note
/// written far from the screen that shows it.
fn note_text(raw: &str, lang: &str) -> String {
    let (key, arg) = match raw.split_once('\u{1f}') {
        Some((k, a)) => (k, a),
        None => (raw, ""),
    };
    let text = catalog().tr(lang, key);
    if text == key {
        // No entry: show what the collector said rather than a bare key.
        return raw.replace('\u{1f}', ": ");
    }
    text.replace("{}", arg)
}

export_module!(SysInfo);

#[cfg(test)]
mod tests;
