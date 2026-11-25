//! The layout catalog, its order, the find match, and the text on a layout page.
//!
//! Files come before built-ins. A file with a modification time is newest first.
//! Built-ins are `default`, then `compact`, then the rest by name.

use zellij_tile::prelude::{LayoutInfo, LayoutMetadata, LayoutParsingError, LayoutWithError};

#[cfg(test)]
use zellij_tile::prelude::{PaneMetadata, TabMetadata};

pub const BUILT_IN_NAMES: [&str; 5] = [
    "default",
    "compact",
    "classic",
    "strider",
    "disable-status-bar",
];

const LAYOUT_DOCS: &str = "https://zellij.dev/documentation/creating-a-layout.html";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    File,
    Builtin,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Current,
    All,
}

impl Scope {
    pub fn toggle(self) -> Self {
        match self {
            Scope::Current => Scope::All,
            Scope::All => Scope::Current,
        }
    }

    pub fn current_tab_only(self) -> bool {
        self == Scope::Current
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub origin: Origin,
    pub modified: Option<i64>,
    pub broken: bool,
    pub error_short: String,
    pub error_full: String,
    pub metadata: Option<LayoutMetadata>,
    pub info: Option<LayoutInfo>,
}

impl Entry {
    pub fn builtin(name: &str) -> Self {
        Self {
            name: name.to_string(),
            origin: Origin::Builtin,
            modified: None,
            broken: false,
            error_short: String::new(),
            error_full: String::new(),
            metadata: None,
            info: Some(LayoutInfo::BuiltIn(name.to_string())),
        }
    }

    pub fn can_open(&self) -> bool {
        self.info.is_some()
    }

    pub fn can_change_file(&self) -> bool {
        self.origin == Origin::File
    }
}

pub fn seeded_builtins() -> Vec<Entry> {
    let mut entries: Vec<Entry> = BUILT_IN_NAMES.iter().copied().map(Entry::builtin).collect();
    sort_entries(&mut entries);
    entries
}

pub fn catalog_from(
    layouts: Vec<LayoutInfo>,
    errors: Vec<LayoutWithError>,
    now: i64,
) -> Vec<Entry> {
    let _ = now;
    let mut entries = Vec::new();
    for info in layouts {
        entries.push(entry_from_info(info));
    }
    for error in errors {
        entries.push(entry_from_error(error));
    }
    sort_entries(&mut entries);
    entries
}

fn entry_from_info(info: LayoutInfo) -> Entry {
    match info {
        LayoutInfo::BuiltIn(name) => Entry::builtin(&name),
        LayoutInfo::File(name, metadata) => {
            let modified = metadata.update_time.parse::<i64>().ok();
            Entry {
                name,
                origin: Origin::File,
                modified,
                broken: false,
                error_short: String::new(),
                error_full: String::new(),
                info: Some(LayoutInfo::File(String::new(), metadata.clone())),
                metadata: Some(metadata),
            }
        }
        LayoutInfo::Url(url) => {
            let name = url.rsplit('/').next().unwrap_or(&url).to_string();
            Entry {
                name,
                origin: Origin::Other,
                modified: None,
                broken: false,
                error_short: String::new(),
                error_full: String::new(),
                metadata: None,
                info: Some(LayoutInfo::Url(url)),
            }
        }
        LayoutInfo::Stringified(text) => Entry {
            name: "raw".to_string(),
            origin: Origin::Other,
            modified: None,
            broken: false,
            error_short: String::new(),
            error_full: String::new(),
            metadata: None,
            info: Some(LayoutInfo::Stringified(text)),
        },
    }
}

fn entry_from_error(error: LayoutWithError) -> Entry {
    let (error_short, error_full) = explain(&error.error);
    Entry {
        name: error.layout_name,
        origin: Origin::File,
        modified: None,
        broken: true,
        error_short,
        error_full,
        metadata: None,
        info: None,
    }
}

/// The file name stored on `LayoutInfo::File` is filled when the entry is opened.
pub fn layout_info(entry: &Entry) -> Option<LayoutInfo> {
    match &entry.info {
        Some(LayoutInfo::File(_, metadata)) => {
            Some(LayoutInfo::File(entry.name.clone(), metadata.clone()))
        }
        other => other.clone(),
    }
}

pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        cmp_entry(a, b).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn cmp_entry(a: &Entry, b: &Entry) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.origin == Origin::Builtin, b.origin == Origin::Builtin) {
        (false, true) => Ordering::Less,
        (true, false) => Ordering::Greater,
        (true, true) => builtin_rank(&a.name)
            .cmp(&builtin_rank(&b.name))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        (false, false) => match (a.modified, b.modified) {
            (Some(a_time), Some(b_time)) => b_time.cmp(&a_time),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        },
    }
}

fn builtin_rank(name: &str) -> u8 {
    match name.to_lowercase().as_str() {
        "default" => 0,
        "compact" => 1,
        _ => 2,
    }
}

pub fn age(modified: Option<i64>, builtin: bool, now: i64) -> String {
    if builtin {
        return "Built-in".to_string();
    }
    let Some(stamp) = modified else {
        return String::new();
    };
    let diff = now.saturating_sub(stamp);
    if diff < 60 {
        "now".to_string()
    } else if diff < 3_600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86_400 {
        format!("{}h ago", diff / 3_600)
    } else {
        format!("{}d ago", diff / 86_400)
    }
}

pub fn scope_of(entry: &Entry) -> Scope {
    if entry.origin == Origin::Builtin {
        return Scope::Current;
    }
    match entry.metadata.as_ref().map(|metadata| metadata.tabs.len()) {
        Some(1) => Scope::Current,
        _ => Scope::All,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub index: usize,
    pub hits: Vec<usize>,
}

pub fn filter(entries: &[Entry], query: &str) -> Vec<Found> {
    if query.is_empty() {
        return (0..entries.len())
            .map(|index| Found {
                index,
                hits: Vec::new(),
            })
            .collect();
    }
    let mut scored = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if let Some((score, hits)) = fuzzy(&entry.name, query) {
            scored.push((score, index, hits));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .map(|(_, index, hits)| Found { index, hits })
        .collect()
}

fn fuzzy(name: &str, query: &str) -> Option<(i64, Vec<usize>)> {
    let name_chars: Vec<char> = name.chars().collect();
    let query_chars: Vec<char> = query.chars().flat_map(|ch| ch.to_lowercase()).collect();
    if query_chars.is_empty() {
        return Some((0, Vec::new()));
    }
    let mut hits = Vec::new();
    let mut query_index = 0;
    let mut score = 0i64;
    let mut previous = None;
    let mut started = None;
    for (index, ch) in name_chars.iter().enumerate() {
        if query_index >= query_chars.len() {
            break;
        }
        if ch.to_lowercase().next() == Some(query_chars[query_index]) {
            if started.is_none() {
                started = Some(index as i64);
            }
            score += if previous == Some(index.wrapping_sub(1)) {
                8
            } else {
                2
            };
            hits.push(index);
            previous = Some(index);
            query_index += 1;
        }
    }
    if query_index < query_chars.len() {
        return None;
    }
    score -= started.unwrap_or(0);
    Some((score, hits))
}

pub fn detail_lines(entry: &Entry) -> Vec<String> {
    if entry.broken {
        return vec![entry.error_short.clone()];
    }
    if entry.origin == Origin::Builtin {
        return vec![format!("{} is a built-in layout.", entry.name)];
    }
    let Some(metadata) = entry.metadata.as_ref() else {
        return vec!["This layout has no pane list.".to_string()];
    };
    let mut lines = Vec::new();
    if metadata.tabs.len() > 1 {
        lines.push("Tabs".to_string());
        for (index, tab) in metadata.tabs.iter().enumerate() {
            let name = tab
                .name
                .clone()
                .unwrap_or_else(|| format!("Tab {}", index + 1));
            lines.push(format!("  {name}"));
        }
    }
    let (named, terminals) = panes_of(metadata);
    if named.is_empty() && terminals == 0 && lines.is_empty() {
        return vec!["This layout has no panes to list.".to_string()];
    }
    if !named.is_empty() || terminals > 0 {
        lines.push("Panes".to_string());
        for name in &named {
            lines.push(format!("  {name}"));
        }
        if terminals > 0 {
            let line = if named.is_empty() {
                format!("  {terminals} Terminals")
            } else {
                format!("  +{terminals} Terminals")
            };
            lines.push(line);
        }
    }
    lines
}

fn panes_of(metadata: &LayoutMetadata) -> (Vec<String>, usize) {
    let mut named = Vec::new();
    let mut terminals = 0usize;
    for tab in &metadata.tabs {
        for pane in &tab.panes {
            if pane.is_builtin_plugin {
                continue;
            }
            match &pane.name {
                Some(name) => named.push(name.clone()),
                None => terminals += 1,
            }
        }
    }
    (named, terminals)
}

pub fn explain(error: &LayoutParsingError) -> (String, String) {
    match error {
        LayoutParsingError::KdlError {
            kdl_error,
            file_name,
            source_code,
        } => {
            let short = format!("Layout error: {}", kdl_error.error_message);
            let mut full = vec![
                file_name.clone(),
                kdl_error.error_message.clone(),
                String::new(),
            ];
            full.extend(excerpt(source_code, kdl_error.offset));
            full.push(String::new());
            full.push(
                kdl_error
                    .help_message
                    .clone()
                    .unwrap_or_else(|| LAYOUT_DOCS.to_string()),
            );
            (short, full.join("\n"))
        },
        LayoutParsingError::SyntaxError => (
            "KDL parsing error".to_string(),
            [
                "The layout file could not be read as KDL.",
                "A node can be missing its closing semicolon.",
                "An argument can be missing its quotation marks.",
                "A property can be missing the equals sign, or an extra equals sign can sit on a child.",
                LAYOUT_DOCS,
            ]
            .join("\n"),
        ),
    }
}

fn excerpt(source: &str, offset: Option<usize>) -> Vec<String> {
    let Some(offset) = offset else {
        return Vec::new();
    };
    let mut start = 0usize;
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let end = start + line.len();
        if offset < end {
            let shown = line.trim_end_matches(['\n', '\r']);
            let column = shown
                .char_indices()
                .take_while(|(byte, _)| *byte < offset - start)
                .count();
            return vec![
                format!("{:>4} | {shown}", index + 1),
                format!("     | {}^{}", " ".repeat(column), ""),
            ];
        }
        start = end;
    }
    Vec::new()
}

pub fn fit_lines(lines: &[String], rows: usize) -> Vec<String> {
    if rows == 0 || lines.len() <= rows {
        return lines.to_vec();
    }
    if rows == 1 {
        return vec![format!("... {} lines omitted ...", lines.len())];
    }
    let room = rows - 1;
    let head = ((room as f32) * 0.6).ceil() as usize;
    let head = head.min(room);
    let tail = room - head;
    let omitted = lines.len().saturating_sub(head + tail);
    let mut out = lines.iter().take(head).cloned().collect::<Vec<_>>();
    out.push(format!("... {omitted} lines omitted ..."));
    if tail > 0 {
        out.extend(lines.iter().skip(lines.len() - tail).cloned());
    }
    out
}

#[cfg(test)]
pub fn file_metadata(tabs: Vec<TabMetadata>, modified: i64) -> LayoutMetadata {
    LayoutMetadata {
        tabs,
        creation_time: modified.to_string(),
        update_time: modified.to_string(),
    }
}

#[cfg(test)]
pub fn pane(name: Option<&str>, builtin: bool) -> PaneMetadata {
    PaneMetadata {
        name: name.map(str::to_string),
        is_plugin: name.is_some() && builtin,
        is_builtin_plugin: builtin,
    }
}

#[cfg(test)]
pub fn tab(name: Option<&str>, panes: Vec<PaneMetadata>) -> TabMetadata {
    TabMetadata {
        name: name.map(str::to_string),
        panes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, modified: Option<i64>, tabs: usize) -> Entry {
        let metadata = file_metadata(
            (0..tabs)
                .map(|index| tab(Some(&format!("Tab {}", index + 1)), vec![pane(None, false)]))
                .collect(),
            modified.unwrap_or(0),
        );
        Entry {
            name: name.to_string(),
            origin: Origin::File,
            modified,
            broken: false,
            error_short: String::new(),
            error_full: String::new(),
            info: Some(LayoutInfo::File(name.to_string(), metadata.clone())),
            metadata: Some(metadata),
        }
    }

    #[test]
    fn files_sort_ahead_of_builtins_and_newest_first() {
        let mut entries = vec![
            Entry::builtin("strider"),
            file("old", Some(10), 1),
            Entry::builtin("default"),
            file("new", Some(50), 2),
            file("undated", None, 1),
            Entry::builtin("compact"),
        ];
        sort_entries(&mut entries);
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            ["new", "old", "undated", "default", "compact", "strider"]
        );
    }

    #[test]
    fn empty_query_keeps_catalog_order() {
        let entries = seeded_builtins();
        let found = filter(&entries, "");
        let names: Vec<_> = found
            .iter()
            .map(|item| entries[item.index].name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "default",
                "compact",
                "classic",
                "disable-status-bar",
                "strider"
            ]
        );
    }

    #[test]
    fn fuzzy_query_prefers_the_tighter_match() {
        let entries = seeded_builtins();
        let found = filter(&entries, "cmp");
        assert_eq!(entries[found[0].index].name, "compact");
        assert!(!found[0].hits.is_empty());
    }

    #[test]
    fn scope_follows_builtin_and_tab_count() {
        assert_eq!(scope_of(&Entry::builtin("default")), Scope::Current);
        assert_eq!(scope_of(&file("one", Some(1), 1)), Scope::Current);
        assert_eq!(scope_of(&file("two", Some(1), 2)), Scope::All);
    }

    #[test]
    fn age_labels_use_minutes_hours_and_days() {
        assert_eq!(age(Some(1_000), false, 1_030), "now");
        assert_eq!(age(Some(1_000), false, 1_120), "2m ago");
        assert_eq!(age(Some(1_000), false, 1_000 + 7_200), "2h ago");
        assert_eq!(age(Some(1_000), false, 1_000 + 86_400 * 3), "3d ago");
        assert_eq!(age(None, true, 0), "Built-in");
    }

    #[test]
    fn detail_lists_tabs_only_when_there_are_several() {
        let one = file("one", Some(1), 1);
        let lines = detail_lines(&one);
        assert!(lines.iter().any(|line| line == "Panes"));
        assert!(!lines.iter().any(|line| line == "Tabs"));

        let two = file("two", Some(1), 2);
        let lines = detail_lines(&two);
        assert!(lines.iter().any(|line| line == "Tabs"));
        assert!(lines.iter().any(|line| line == "  Tab 1"));
    }

    #[test]
    fn detail_skips_builtin_plugins_and_counts_shells() {
        let metadata = file_metadata(
            vec![tab(
                Some("Main"),
                vec![
                    pane(Some("tab-bar"), true),
                    pane(Some("vim"), false),
                    pane(None, false),
                ],
            )],
            1,
        );
        let entry = Entry {
            name: "work".to_string(),
            origin: Origin::File,
            modified: Some(1),
            broken: false,
            error_short: String::new(),
            error_full: String::new(),
            info: None,
            metadata: Some(metadata),
        };
        let lines = detail_lines(&entry);
        assert!(lines.iter().any(|line| line == "  vim"));
        assert!(lines.iter().any(|line| line == "  +1 Terminals"));
        assert!(!lines.iter().any(|line| line.contains("tab-bar")));
    }

    #[test]
    fn a_long_diagnostic_omits_the_middle() {
        let lines: Vec<_> = (0..10).map(|n| format!("line {n}")).collect();
        let fitted = fit_lines(&lines, 4);
        assert_eq!(fitted.len(), 4);
        assert!(fitted.iter().any(|line| line.contains("omitted")));
        assert_eq!(fitted[0], "line 0");
        assert_eq!(fitted[3], "line 9");
    }
}
