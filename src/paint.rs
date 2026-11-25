//! The three pages. The last rows are the key bar. The rows above them are the page.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::chrome::{Chrome, Ink};
use crate::controls::{paint_row, Click, Piece};
use crate::model::{age, detail_lines, Entry, Found, Scope};

pub struct Hit {
    pub y: usize,
    pub x0: usize,
    pub x1: usize,
    pub click: Click,
}

pub struct Frame {
    pub lines: Vec<String>,
    pub hits: Vec<Hit>,
    pub cursor: Option<(usize, usize)>,
}

pub struct Pointer {
    pub y: Option<usize>,
    pub x: Option<usize>,
}

pub struct ListView<'a> {
    pub entries: &'a [Entry],
    pub found: &'a [Found],
    pub selected: usize,
    pub finding: bool,
    pub query: &'a str,
    pub cursor: usize,
    pub keep_terminals: bool,
    pub keep_plugins: bool,
    pub scope: Scope,
    pub notice: Option<&'a str>,
    pub now: i64,
}

pub fn paint_list(
    view: &ListView<'_>,
    chrome: &Chrome,
    rows: usize,
    cols: usize,
    pointer: Pointer,
) -> Frame {
    let mut lines = vec![String::new(); rows];
    let mut hits = Vec::new();
    let mut cursor = None;
    if rows == 0 || cols == 0 {
        return Frame {
            lines,
            hits,
            cursor,
        };
    }
    let bars = list_bars(view);
    let footer = bars.len().min(rows);
    let mut y = 0usize;
    if view.notice.is_some() && y < rows.saturating_sub(footer) {
        put(
            &mut lines,
            y,
            body_line(view.notice.unwrap_or(""), cols, chrome),
        );
        y += 1;
    }
    if y < rows.saturating_sub(footer) {
        let (line, cursor_x) = find_line(view, chrome, cols);
        put(&mut lines, y, line);
        if view.finding {
            cursor = Some((cursor_x.min(cols.saturating_sub(1)), y));
        }
        y += 1;
    }
    let body_top = y + 1;
    let body_height = rows.saturating_sub(footer).saturating_sub(body_top);
    let start = window(view.found.len(), view.selected, body_height);
    for slot in 0..body_height {
        let y = body_top + slot;
        let Some(found) = view.found.get(start + slot) else {
            put(&mut lines, y, body_line("", cols, chrome));
            continue;
        };
        let entry = &view.entries[found.index];
        let selected = start + slot == view.selected;
        let hover = pointer.y == Some(y) && !selected;
        put(
            &mut lines,
            y,
            entry_line(entry, &found.hits, selected, hover, view.now, chrome, cols),
        );
        hits.push(Hit {
            y,
            x0: 0,
            x1: cols,
            click: Click::Row(start + slot),
        });
    }
    place_bars(&mut lines, &mut hits, &bars, rows, cols, chrome, &pointer);
    Frame {
        lines,
        hits,
        cursor,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn paint_page(
    notice: Option<&str>,
    body: &[String],
    name_cursor_x: Option<usize>,
    bars: &[Vec<Piece>],
    chrome: &Chrome,
    rows: usize,
    cols: usize,
    pointer: Pointer,
) -> Frame {
    let mut lines = vec![String::new(); rows];
    let mut hits = Vec::new();
    let mut cursor = None;
    if rows == 0 || cols == 0 {
        return Frame {
            lines,
            hits,
            cursor,
        };
    }
    let footer = bars.len().min(rows);
    let mut y = 0usize;
    if let Some(notice) = notice {
        if y < rows.saturating_sub(footer) {
            put(&mut lines, y, body_line(notice, cols, chrome));
            y += 1;
        }
    }
    let body_height = rows.saturating_sub(footer).saturating_sub(y);
    let fitted = crate::model::fit_lines(body, body_height);
    for slot in 0..body_height {
        let text = fitted.get(slot).map(String::as_str).unwrap_or("");
        put(&mut lines, y + slot, body_line(text, cols, chrome));
    }
    if let Some(x) = name_cursor_x {
        cursor = Some((x.min(cols.saturating_sub(1)), y));
    }
    place_bars(&mut lines, &mut hits, bars, rows, cols, chrome, &pointer);
    Frame {
        lines,
        hits,
        cursor,
    }
}

pub fn name_line(text: &str, editing: bool, cursor: usize) -> (String, Option<usize>) {
    if text.is_empty() && !editing {
        return ("Name Random".to_string(), None);
    }
    let shown: String = text.chars().take(cursor).collect();
    (
        format!("Name {text}"),
        if editing {
            Some(UnicodeWidthStr::width("Name ") + UnicodeWidthStr::width(shown.as_str()))
        } else {
            None
        },
    )
}

fn list_bars(view: &ListView<'_>) -> Vec<Vec<Piece>> {
    let keys = !view.finding;
    let mut bars = vec![
        scope_pieces(view.scope, keys),
        unmatched_pieces(
            "Unmatched Terminals",
            "T",
            view.keep_terminals,
            keys,
            Click::ToggleTerminals,
            Click::Terminals(false),
            Click::Terminals(true),
        ),
        unmatched_pieces(
            "Unmatched Plugins",
            "P",
            view.keep_plugins,
            keys,
            Click::TogglePlugins,
            Click::Plugins(false),
            Click::Plugins(true),
        ),
    ];
    bars.push(if view.finding {
        vec![
            key("↑", "Up", Click::Up),
            key("↓", "Down", Click::Down),
            key("TAB", "Complete", Click::Complete),
            key("ENTER", "Open", Click::Open),
            key("⌥W", "Apply", Click::Apply),
            key("⌥→", "Info", Click::Info),
            key("ESC", "Clear", Click::Clear),
        ]
    } else {
        vec![
            key("↑", "Up", Click::Up),
            key("↓", "Down", Click::Down),
            key("/", "Find", Click::Find),
            key("ENTER", "Open", Click::Open),
            key("⌥W", "Apply", Click::Apply),
            key("→", "Info", Click::Info),
            key("N", "New", Click::New),
            key("ESC", "Close", Click::Close),
        ]
    });
    bars
}

pub fn unmatched_pieces(
    title: &'static str,
    key_glyph: &'static str,
    keep: bool,
    key_shown: bool,
    toggle: Click,
    close: Click,
    kept: Click,
) -> Vec<Piece> {
    let mut row = Vec::new();
    if key_shown {
        row.push(key_short(key_glyph, title, short_title(title), toggle));
    } else {
        row.push(Piece::Title(title));
    }
    row.extend([choice("Close", !keep, close), choice("Keep", keep, kept)]);
    row
}

pub fn scope_pieces(scope: Scope, key_shown: bool) -> Vec<Piece> {
    let mut row = Vec::new();
    if key_shown {
        row.push(key_short("A", "Apply To Tab", "Apply", Click::ScopeCycle));
    } else {
        row.push(Piece::Title("Apply To Tab"));
    }
    row.extend([
        choice(
            "Current",
            scope == Scope::Current,
            Click::Scope(Scope::Current),
        ),
        choice("All Tabs", scope == Scope::All, Click::Scope(Scope::All)),
    ]);
    row
}

pub fn one_bars(file: bool, broken: bool, renaming: bool) -> Vec<Vec<Piece>> {
    if renaming {
        return vec![vec![
            key("ENTER", "Save", Click::Save),
            key("ESC", "Cancel", Click::Back),
        ]];
    }
    let mut row = Vec::new();
    if file {
        row.push(key("E", "Edit", Click::Edit));
        row.push(key("R", "Rename", Click::Rename));
        row.push(key("DEL", "Delete", Click::Delete));
    }
    if broken {
        row.push(key("M", "Debug", Click::Debug));
    }
    row.push(key("ESC", "Back", Click::Back));
    vec![row]
}

pub fn new_choice_row(session: bool, letters_are_keys: bool) -> Vec<Piece> {
    if letters_are_keys {
        vec![
            choice("S Session", session, Click::Session),
            choice("I Import", !session, Click::Import),
        ]
    } else {
        vec![
            choice("Session", session, Click::Session),
            choice("Import", !session, Click::Import),
        ]
    }
}

pub fn new_tab_row(current_only: bool) -> Vec<Piece> {
    vec![
        key("TAB", "Tabs", Click::ScopeCycle),
        choice("All Tabs", !current_only, Click::Scope(Scope::All)),
        choice("Current Tab", current_only, Click::Scope(Scope::Current)),
    ]
}

pub fn new_action_row(session: bool, can_save: bool, letters_are_keys: bool) -> Vec<Piece> {
    let mut row = Vec::new();
    if session {
        row.push(key("U", "Refresh", Click::Refresh));
    }
    if can_save {
        if letters_are_keys {
            row.push(key("R", "Name", Click::Name));
        } else {
            row.push(choice("Name", false, Click::Name));
        }
        row.push(key("ENTER", "Save", Click::Save));
    }
    row.push(key("ESC", "Back", Click::Back));
    row
}

fn key(key: &'static str, label: &'static str, click: Click) -> Piece {
    key_short(key, label, label, click)
}

fn key_short(key: &'static str, label: &'static str, short: &'static str, click: Click) -> Piece {
    Piece::Key {
        key,
        label,
        short,
        click,
    }
}

fn short_title(title: &str) -> &'static str {
    match title {
        "Unmatched Terminals" => "Terminals",
        "Unmatched Plugins" => "Plugins",
        "Apply To Tab" => "Apply",
        _ => "",
    }
}

fn choice(label: &'static str, on: bool, click: Click) -> Piece {
    Piece::Choice { label, on, click }
}

fn place_bars(
    lines: &mut [String],
    hits: &mut Vec<Hit>,
    bars: &[Vec<Piece>],
    rows: usize,
    cols: usize,
    chrome: &Chrome,
    pointer: &Pointer,
) {
    let start = rows.saturating_sub(bars.len());
    for (offset, pieces) in bars.iter().enumerate() {
        let y = start + offset;
        if y >= rows {
            break;
        }
        let hover_x = if pointer.y == Some(y) {
            pointer.x
        } else {
            None
        };
        let (ansi, row_hits) = paint_row(pieces, cols, chrome, hover_x);
        lines[y] = ansi;
        for (x0, x1, click) in row_hits {
            hits.push(Hit { y, x0, x1, click });
        }
    }
}

fn find_line(view: &ListView<'_>, chrome: &Chrome, cols: usize) -> (String, usize) {
    let prompt = "Find: ";
    let typed: String = view.query.chars().take(view.cursor).collect();
    let cursor_x = UnicodeWidthStr::width(prompt) + UnicodeWidthStr::width(typed.as_str());
    let text = format!("{prompt}{}", view.query);
    (colored(&text, cols, chrome.light, chrome.bar), cursor_x)
}

fn entry_line(
    entry: &Entry,
    hits: &[usize],
    selected: bool,
    hover: bool,
    now: i64,
    chrome: &Chrome,
    cols: usize,
) -> String {
    let age = age(
        entry.modified,
        entry.origin == crate::model::Origin::Builtin,
        now,
    );
    let age_width = UnicodeWidthStr::width(age.as_str());
    let gap = if age_width == 0 { 0 } else { 1 };
    let name_cols = cols.saturating_sub(age_width + gap);
    let name = truncate(&entry.name, name_cols);
    if selected {
        let mut text = name;
        let pad = name_cols.saturating_sub(UnicodeWidthStr::width(text.as_str()));
        text.push_str(&" ".repeat(pad));
        if gap > 0 {
            text.push(' ');
            text.push_str(&age);
        }
        return colored(&text, cols, chrome.selected_fg, chrome.selected_bg);
    }
    let fg = chrome.light;
    let bg = if hover { chrome.hover } else { chrome.bar };
    let mut out = String::from("\u{1b}[0m");
    let mut used = 0usize;
    for (index, ch) in name.chars().enumerate() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > name_cols {
            break;
        }
        let ink = if hits.contains(&index) {
            chrome.alt
        } else {
            fg
        };
        out.push_str(&format!("\u{1b}[{};{}m{ch}", ink.fg(), bg.bg()));
        used += width;
    }
    if used < cols {
        let rest = if age_width == 0 {
            " ".repeat(cols - used)
        } else {
            let between = cols - used - age_width;
            format!("{}{age}", " ".repeat(between))
        };
        out.push_str(&format!("\u{1b}[{};{}m{rest}", fg.fg(), bg.bg()));
    }
    out.push_str("\u{1b}[0m");
    out
}

fn body_line(text: &str, cols: usize, chrome: &Chrome) -> String {
    colored(text, cols, chrome.light, chrome.bar)
}

fn colored(text: &str, cols: usize, fg: Ink, bg: Ink) -> String {
    let shown = truncate(text, cols);
    let gap = cols.saturating_sub(UnicodeWidthStr::width(shown.as_str()));
    format!(
        "\u{1b}[0m\u{1b}[{};{}m{shown}{}\u{1b}[0m",
        fg.fg(),
        bg.bg(),
        " ".repeat(gap)
    )
}

fn truncate(text: &str, cols: usize) -> String {
    if UnicodeWidthStr::width(text) <= cols {
        return text.to_string();
    }
    let budget = cols.saturating_sub(3);
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > budget {
            break;
        }
        out.push(ch);
        used += width;
    }
    out.push_str("...");
    out
}

fn window(len: usize, selected: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    let mut start = selected.saturating_sub(height / 2);
    if start + height > len {
        start = len - height;
    }
    start
}

fn put(lines: &mut [String], y: usize, text: String) {
    if y < lines.len() {
        lines[y] = text;
    }
}

pub fn one_body(entry: &Entry, debug: bool, renaming: bool, name: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if renaming {
        lines.push(name_line(name, true, 0).0);
    }
    if debug && entry.broken {
        lines.extend(entry.error_full.lines().map(str::to_string));
        return lines;
    }
    lines.extend(detail_lines(entry));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::Chrome;
    use crate::controls::visible;
    use crate::model::seeded_builtins;

    #[test]
    fn the_list_shows_builtin_names_and_open() {
        let entries = seeded_builtins();
        let found = crate::model::filter(&entries, "");
        let frame = paint_list(
            &ListView {
                entries: &entries,
                found: &found,
                selected: 0,
                finding: true,
                query: "",
                cursor: 0,
                keep_terminals: false,
                keep_plugins: false,
                scope: Scope::Current,
                notice: None,
                now: 0,
            },
            &Chrome::default(),
            22,
            72,
            Pointer { y: None, x: None },
        );
        let text = frame
            .lines
            .iter()
            .map(|line| visible(line))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("default"));
        assert!(text.contains("compact"));
        assert!(text.contains("Open"));
        assert!(text.contains("Apply"));
        assert!(text.contains("Find:"));
        assert!(text.contains("Apply To Tab"));
        assert!(text.contains("Unmatched Terminals"));
        assert!(text.contains("Unmatched Plugins"));
        assert!(text.contains("Close"));
        assert!(text.contains("Keep"));
        assert!(!text.contains("T Unmatched"));
        assert!(!text.contains("P Unmatched"));
        assert!(!text.contains("A Apply"));
        assert!(text.contains("Current"));
        assert!(frame.cursor.is_some());
    }
}
