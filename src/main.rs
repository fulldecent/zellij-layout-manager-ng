//! Three pages for layouts: the list, one layout, and a new layout.
//!
//! The list opens or applies the selection. The layout page reads, edits,
//! renames, deletes, or shows a parse diagnostic. New records the session or
//! imports pasted KDL.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use zellij_tile::prelude::*;

use crate::chrome::{chrome_from, Chrome};
use crate::controls::Click;
use crate::line::{Edit, Line};
use crate::model::{
    catalog_from, detail_lines, explain, filter, layout_info, scope_of, seeded_builtins,
    sort_entries, Entry, Found, Origin, Scope,
};
use crate::paint::{
    name_line, new_action_row, new_choice_row, new_tab_row, one_bars, one_body, paint_list,
    paint_page, ListView, Pointer,
};

mod chrome;
mod controls;
mod line;
mod model;
mod paint;

struct State {
    entries: Vec<Entry>,
    found: Vec<Found>,
    selected: usize,
    finding: bool,
    query: Line,
    /// Same start as the built-in layout manager: unmatched panes are closed.
    keep_terminals: bool,
    keep_plugins: bool,
    scope: Scope,
    scope_name: Option<String>,
    page: Page,
    granted: bool,
    notice: Option<String>,
    chrome: Chrome,
    rows: usize,
    cols: usize,
    pointer: Option<(usize, usize)>,
    hits: Vec<paint::Hit>,
    /// The name last written into the floating pane border.
    border: Option<String>,
}

enum Page {
    List,
    One(OnePage),
    New(NewPage),
}

struct OnePage {
    name: String,
    origin: Origin,
    broken: bool,
    renaming: bool,
    name_line: Line,
    debug: bool,
}

struct NewPage {
    kind: NewKind,
    name_line: Line,
    editing: bool,
    current_tab_only: bool,
    session_kdl: Option<String>,
    session_meta: Option<LayoutMetadata>,
    pasted: Option<String>,
    import_meta: Option<LayoutMetadata>,
    import_error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NewKind {
    Session,
    Import,
}

impl Default for State {
    fn default() -> Self {
        let entries = seeded_builtins();
        Self {
            entries,
            found: Vec::new(),
            selected: 0,
            finding: true,
            query: Line::new(),
            keep_terminals: false,
            keep_plugins: false,
            scope: Scope::Current,
            scope_name: None,
            page: Page::List,
            granted: false,
            notice: None,
            chrome: Chrome::default(),
            rows: 0,
            cols: 0,
            pointer: None,
            hits: Vec::new(),
            border: None,
        }
    }
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::Key,
            EventType::Mouse,
            EventType::ModeUpdate,
            EventType::PermissionRequestResult,
            EventType::PastedText,
            EventType::AvailableLayoutInfo,
        ]);
        set_selectable(true);
        self.sync();
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                self.granted = true;
                if self.notice.as_deref() == Some(GRANT) {
                    self.notice = None;
                }
                self.retitle();
                if let Page::New(page) = &self.page {
                    if page.kind == NewKind::Session && page.session_kdl.is_none() {
                        self.dump_session();
                    }
                }
                true
            }
            Event::PermissionRequestResult(_) => {
                self.granted = false;
                self.notice = Some(GRANT.to_string());
                true
            }
            Event::ModeUpdate(info) => {
                let next = chrome_from(&info.style);
                if next == self.chrome {
                    false
                } else {
                    self.chrome = next;
                    true
                }
            }
            Event::AvailableLayoutInfo(layouts, errors) => {
                self.replace_catalog(catalog_from(layouts, errors, unix_now()));
                true
            }
            Event::PastedText(text) => self.paste(text),
            Event::Key(key) => self.on_key(key),
            Event::Mouse(mouse) => self.on_mouse(mouse),
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.rows = rows;
        self.cols = cols;
        if rows == 0 || cols == 0 {
            return;
        }
        self.sync();
        self.recover_missing_page();
        self.retitle();
        let pointer = Pointer {
            y: self.pointer.map(|(y, _)| y),
            x: self.pointer.map(|(_, x)| x),
        };
        let frame = match &self.page {
            Page::List => paint_list(
                &ListView {
                    entries: &self.entries,
                    found: &self.found,
                    selected: self.selected,
                    finding: self.finding,
                    query: self.query.text(),
                    cursor: self.query.cursor(),
                    keep_terminals: self.keep_terminals,
                    keep_plugins: self.keep_plugins,
                    scope: self.scope,
                    notice: self.list_notice(),
                    now: unix_now(),
                },
                &self.chrome,
                rows,
                cols,
                pointer,
            ),
            Page::One(_) => self.paint_one(rows, cols, pointer),
            Page::New(_) => self.paint_new(rows, cols, pointer),
        };
        match frame.cursor {
            Some((x, y)) => show_cursor(Some((x, y))),
            None => show_cursor(None),
        }
        self.hits = frame.hits;
        print!("\u{1b}[?7l");
        for (y, line) in frame.lines.iter().enumerate() {
            print!("\u{1b}[{};1H{line}", y + 1);
        }
        print!("\u{1b}[?7h");
        let _ = io::stdout().flush();
    }
}

const GRANT: &str = "Grant Read Application State and Change Application State.";

impl State {
    fn on_key(&mut self, key: KeyWithModifier) -> bool {
        match &self.page {
            Page::List => self.list_key(&key),
            Page::One(_) => self.one_key(&key),
            Page::New(_) => self.new_key(&key),
        }
    }

    fn list_key(&mut self, key: &KeyWithModifier) -> bool {
        if self.finding {
            if is_apply(key) {
                self.apply_selected();
                return true;
            }
            if is_alt_right(key) {
                self.open_info();
                return true;
            }
            if key.has_no_modifiers() && matches!(key.bare_key, BareKey::Up) {
                self.move_selection(-1);
                return true;
            }
            if key.has_no_modifiers() && matches!(key.bare_key, BareKey::Down) {
                self.move_selection(1);
                return true;
            }
            return match self.query.handle(key) {
                Edit::Changed => true,
                Edit::Submit => {
                    self.open_selected();
                    true
                }
                Edit::Cancel => {
                    self.finding = false;
                    true
                }
                Edit::Complete => {
                    self.complete_query();
                    true
                }
                Edit::Ignored => false,
            };
        }
        if is_apply(key) {
            self.apply_selected();
            return true;
        }
        if !key.has_no_modifiers() {
            return false;
        }
        match key.bare_key {
            BareKey::Up => self.move_selection(-1),
            BareKey::Down => self.move_selection(1),
            BareKey::Char('/') => {
                self.finding = true;
                true
            }
            BareKey::Enter => {
                self.open_selected();
                true
            }
            BareKey::Char('n') | BareKey::Char('N') => {
                self.open_new(NewKind::Session);
                true
            }
            BareKey::Char('t') | BareKey::Char('T') => {
                self.keep_terminals = !self.keep_terminals;
                true
            }
            BareKey::Char('p') | BareKey::Char('P') => {
                self.keep_plugins = !self.keep_plugins;
                true
            }
            BareKey::Char('a') | BareKey::Char('A') => {
                self.scope = self.scope.toggle();
                true
            }
            BareKey::Right => {
                self.open_info();
                true
            }
            BareKey::Esc => {
                close_self();
                true
            }
            _ => false,
        }
    }

    fn one_key(&mut self, key: &KeyWithModifier) -> bool {
        let renaming = matches!(&self.page, Page::One(page) if page.renaming);
        if renaming {
            let edit = {
                let Some(page) = self.page_one_mut() else {
                    return false;
                };
                let edit = page.name_line.handle_form(key);
                let typed = page.name_line.text().to_string();
                (edit, typed)
            };
            return match edit.0 {
                Edit::Changed => true,
                Edit::Submit => {
                    self.commit_rename(edit.1);
                    true
                }
                Edit::Cancel => {
                    if let Page::One(page) = &mut self.page {
                        page.renaming = false;
                    }
                    true
                }
                Edit::Complete | Edit::Ignored => true,
            };
        }
        if !key.has_no_modifiers() {
            return false;
        }
        let file = self.current_entry().is_some_and(Entry::can_change_file);
        let broken = self.current_entry().is_some_and(|entry| entry.broken);
        match key.bare_key {
            BareKey::Esc => {
                if let Page::One(page) = &mut self.page {
                    if page.debug {
                        page.debug = false;
                        return true;
                    }
                }
                self.page = Page::List;
                self.notice = None;
            }
            BareKey::Char('e') | BareKey::Char('E') if file => self.edit_current(),
            BareKey::Char('r') | BareKey::Char('R') if file => self.begin_rename(),
            BareKey::Delete if file => self.delete_current(),
            BareKey::Char('m') | BareKey::Char('M') if broken => {
                if let Page::One(page) = &mut self.page {
                    page.debug = !page.debug;
                }
            }
            _ => return false,
        }
        true
    }

    fn new_key(&mut self, key: &KeyWithModifier) -> bool {
        let editing = matches!(&self.page, Page::New(page) if page.editing);
        if editing {
            let edit = {
                let Some(page) = self.page_new_mut() else {
                    return false;
                };
                page.name_line.handle_form(key)
            };
            return match edit {
                Edit::Changed => true,
                Edit::Submit | Edit::Cancel => {
                    if let Page::New(page) = &mut self.page {
                        page.editing = false;
                    }
                    true
                }
                Edit::Complete | Edit::Ignored => true,
            };
        }
        let importing = matches!(&self.page, Page::New(page) if page.kind == NewKind::Import);
        if importing {
            if !key.has_no_modifiers() {
                return false;
            }
            match key.bare_key {
                BareKey::Esc => {
                    self.page = Page::List;
                    self.notice = None;
                }
                BareKey::Enter => self.save_new(),
                BareKey::Backspace => self.import_backspace(),
                BareKey::Char(ch) if !ch.is_control() => self.import_push(ch),
                _ => return false,
            }
            return true;
        }
        if !key.has_no_modifiers() {
            return false;
        }
        match key.bare_key {
            BareKey::Esc => {
                self.page = Page::List;
                self.notice = None;
            }
            BareKey::Char('s') | BareKey::Char('S') => self.switch_new(NewKind::Session),
            BareKey::Char('i') | BareKey::Char('I') => self.switch_new(NewKind::Import),
            BareKey::Tab => self.toggle_new_tabs(),
            BareKey::Char('u') | BareKey::Char('U') => self.dump_session(),
            BareKey::Char('r') | BareKey::Char('R') => {
                if let Page::New(page) = &mut self.page {
                    page.editing = true;
                }
            }
            BareKey::Enter => self.save_new(),
            _ => return false,
        }
        true
    }

    fn on_mouse(&mut self, mouse: Mouse) -> bool {
        match mouse {
            Mouse::ScrollUp(lines) => self.move_selection(-(lines as isize)),
            Mouse::ScrollDown(lines) => self.move_selection(lines as isize),
            Mouse::Hover(line, column) => {
                self.pointer = Some((line.max(0) as usize, column));
                true
            }
            Mouse::LeftClick(line, column) => {
                let line = line.max(0) as usize;
                self.pointer = Some((line, column));
                if let Some(click) = self.hit_at(line, column) {
                    self.on_click(click);
                }
                true
            }
            _ => false,
        }
    }

    fn on_click(&mut self, click: Click) {
        match click {
            Click::Up => {
                self.move_selection(-1);
            }
            Click::Down => {
                self.move_selection(1);
            }
            Click::Complete => self.complete_query(),
            Click::Open => self.open_selected(),
            Click::Apply => self.apply_selected(),
            Click::Info => self.open_info(),
            Click::New => self.open_new(NewKind::Session),
            Click::Find => self.finding = true,
            Click::Clear => {
                if self.query.text().is_empty() {
                    self.finding = false;
                } else {
                    self.query.clear();
                }
            }
            Click::Close => close_self(),
            Click::ToggleTerminals => self.keep_terminals = !self.keep_terminals,
            Click::Terminals(keep) => self.keep_terminals = keep,
            Click::TogglePlugins => self.keep_plugins = !self.keep_plugins,
            Click::Plugins(keep) => self.keep_plugins = keep,
            Click::ScopeCycle => self.cycle_scope(),
            Click::Scope(scope) => self.set_scope(scope),
            Click::Edit => self.edit_current(),
            Click::Rename => self.begin_rename(),
            Click::Delete => self.delete_current(),
            Click::Debug => {
                if let Page::One(page) = &mut self.page {
                    page.debug = !page.debug;
                }
            }
            Click::Back => self.click_back(),
            Click::Save => self.click_save(),
            Click::Refresh => self.dump_session(),
            Click::Name => {
                if let Page::New(page) = &mut self.page {
                    page.editing = true;
                }
            }
            Click::Session => self.switch_new(NewKind::Session),
            Click::Import => self.switch_new(NewKind::Import),
            Click::Row(index) => {
                if self.selected == index {
                    self.open_info();
                } else {
                    self.selected = index;
                }
            }
        }
    }

    fn click_back(&mut self) {
        match &mut self.page {
            Page::One(page) if page.renaming => page.renaming = false,
            Page::One(page) if page.debug => page.debug = false,
            Page::New(page) if page.editing => page.editing = false,
            _ => {
                self.page = Page::List;
                self.notice = None;
            }
        }
    }

    fn click_save(&mut self) {
        let renaming = match &self.page {
            Page::One(page) if page.renaming => Some(page.name_line.text().to_string()),
            _ => None,
        };
        if let Some(name) = renaming {
            self.commit_rename(name);
            return;
        }
        let editing_new = matches!(&self.page, Page::New(page) if page.editing);
        if editing_new {
            if let Page::New(page) = &mut self.page {
                page.editing = false;
            }
            return;
        }
        if matches!(self.page, Page::New(_)) {
            self.save_new();
        }
    }

    fn cycle_scope(&mut self) {
        if matches!(self.page, Page::New(_)) {
            self.toggle_new_tabs();
        } else {
            self.scope = self.scope.toggle();
        }
    }

    fn set_scope(&mut self, scope: Scope) {
        let retarget = if let Page::New(page) = &mut self.page {
            let current = scope == Scope::Current;
            if page.current_tab_only == current {
                false
            } else {
                page.current_tab_only = current;
                true
            }
        } else {
            self.scope = scope;
            false
        };
        if retarget {
            self.dump_session();
        }
    }

    fn move_selection(&mut self, delta: isize) -> bool {
        let len = self.found.len();
        if len == 0 {
            return true;
        }
        if delta < 0 {
            self.selected = if self.selected == 0 {
                len - 1
            } else {
                self.selected - 1
            };
        } else if self.selected + 1 >= len {
            self.selected = 0;
        } else {
            self.selected += delta as usize;
        }
        true
    }

    fn complete_query(&mut self) {
        if let Some(name) = self.selected_entry().map(|entry| entry.name.clone()) {
            self.query.set(name);
        }
    }

    fn open_info(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        self.page = Page::One(OnePage {
            name: entry.name.clone(),
            origin: entry.origin,
            broken: entry.broken,
            renaming: false,
            name_line: Line::new(),
            debug: false,
        });
        self.notice = None;
    }

    fn open_new(&mut self, kind: NewKind) {
        self.page = Page::New(NewPage {
            kind,
            name_line: Line::new(),
            editing: false,
            current_tab_only: false,
            session_kdl: None,
            session_meta: None,
            pasted: None,
            import_meta: None,
            import_error: None,
        });
        self.notice = None;
        if kind == NewKind::Session {
            self.dump_session();
        }
    }

    fn switch_new(&mut self, kind: NewKind) {
        let was = matches!(&self.page, Page::New(page) if page.kind == kind);
        if let Page::New(page) = &mut self.page {
            page.kind = kind;
            page.editing = false;
        }
        if !was && kind == NewKind::Session {
            let empty = matches!(&self.page, Page::New(page) if page.session_kdl.is_none());
            if empty {
                self.dump_session();
            }
        }
    }

    fn toggle_new_tabs(&mut self) {
        if let Page::New(page) = &mut self.page {
            if page.kind != NewKind::Session {
                return;
            }
            page.current_tab_only = !page.current_tab_only;
        } else {
            return;
        }
        self.dump_session();
    }

    fn open_selected(&mut self) {
        if !self.require_change() {
            return;
        }
        let Some(entry) = self.selected_entry().cloned() else {
            return;
        };
        if !entry.can_open() {
            self.notice = Some("This layout did not parse.".to_string());
            return;
        }
        let Some(info) = layout_info(&entry) else {
            return;
        };
        let rename = entry.origin == Origin::Builtin
            || entry.metadata.as_ref().map(|meta| meta.tabs.len()) == Some(1);
        let ids = new_tabs_with_layout_info(&info);
        if rename {
            if let Some(id) = ids.first() {
                rename_tab_with_id(*id as u64, &entry.name);
            }
        }
        close_self();
    }

    fn apply_selected(&mut self) {
        if !self.require_change() {
            return;
        }
        let Some(entry) = self.selected_entry().cloned() else {
            return;
        };
        if !entry.can_open() {
            self.notice = Some("This layout did not parse.".to_string());
            return;
        }
        let Some(info) = layout_info(&entry) else {
            return;
        };
        override_layout(
            &info,
            self.keep_terminals,
            self.keep_plugins,
            self.scope.current_tab_only(),
            BTreeMap::new(),
        );
        close_self();
    }

    fn edit_current(&mut self) {
        if !self.require_change() {
            return;
        }
        let Some(name) = self.current_entry().map(|entry| entry.name.clone()) else {
            return;
        };
        if !self.current_entry().is_some_and(Entry::can_change_file) {
            self.notice = Some("A built-in layout has no file to edit.".to_string());
            return;
        }
        if let Err(error) = edit_layout(&name, BTreeMap::new()) {
            self.notice = Some(error);
        }
    }

    fn begin_rename(&mut self) {
        let Some(name) = self
            .current_entry()
            .filter(|entry| entry.can_change_file())
            .map(|entry| entry.name.clone())
        else {
            return;
        };
        if let Page::One(page) = &mut self.page {
            page.name_line.set(name);
            page.renaming = true;
            page.debug = false;
        }
    }

    fn commit_rename(&mut self, new_name: String) {
        if new_name.is_empty() {
            self.notice = Some("The layout name cannot be empty.".to_string());
            return;
        }
        if !self.require_change() {
            return;
        }
        let Some(old) = self.current_entry().map(|entry| entry.name.clone()) else {
            return;
        };
        match rename_layout(&old, &new_name) {
            Ok(()) => {
                if let Some(entry) = self.entry_mut(&old, Origin::File) {
                    entry.name = new_name.clone();
                    if let Some(LayoutInfo::File(name, _)) = entry.info.as_mut() {
                        *name = new_name.clone();
                    }
                }
                sort_entries(&mut self.entries);
                if let Page::One(page) = &mut self.page {
                    page.name = new_name;
                    page.renaming = false;
                }
                self.notice = None;
                self.scope_name = None;
            }
            Err(error) => self.notice = Some(error),
        }
    }

    fn delete_current(&mut self) {
        if !self.require_change() {
            return;
        }
        let Some(name) = self
            .current_entry()
            .filter(|entry| entry.can_change_file())
            .map(|entry| entry.name.clone())
        else {
            return;
        };
        self.entries
            .retain(|entry| !(entry.origin == Origin::File && entry.name == name));
        self.page = Page::List;
        self.scope_name = None;
        self.notice = delete_layout(&name).err();
    }

    fn dump_session(&mut self) {
        if !self.require_read() {
            return;
        }
        let current_only = matches!(&self.page, Page::New(page) if page.current_tab_only);
        let dumped = if current_only {
            let index = get_focused_pane_info().map(|(index, _)| index).unwrap_or(0);
            dump_session_layout_for_tab(index)
        } else {
            dump_session_layout()
        };
        match dumped {
            Ok((kdl, Some(metadata))) => {
                if let Page::New(page) = &mut self.page {
                    page.session_kdl = Some(kdl);
                    page.session_meta = Some(metadata);
                }
                self.notice = None;
            }
            Ok((_, None)) => {
                self.notice = Some("The session layout had no pane list.".to_string());
                self.page = Page::List;
            }
            Err(error) => {
                self.notice = Some(error);
                self.page = Page::List;
            }
        }
    }

    fn save_new(&mut self) {
        if !self.require_change() {
            return;
        }
        let Some(page) = self.page_new_ref() else {
            return;
        };
        let typed = page.name_line.text().to_string();
        let (kdl, metadata) = match page.kind {
            NewKind::Session => match (&page.session_kdl, &page.session_meta) {
                (Some(kdl), Some(metadata)) => (kdl.clone(), metadata.clone()),
                _ => {
                    self.notice = Some("The session layout is not ready to save.".to_string());
                    return;
                }
            },
            NewKind::Import => {
                if page.import_error.is_some() || page.import_meta.is_none() {
                    self.notice = Some("Paste a layout that parses.".to_string());
                    return;
                }
                match (&page.pasted, &page.import_meta) {
                    (Some(kdl), Some(metadata)) => (kdl.clone(), metadata.clone()),
                    _ => {
                        self.notice = Some("Paste a layout first.".to_string());
                        return;
                    }
                }
            }
        };
        let name = if typed.is_empty() {
            if !self.require_read() {
                return;
            }
            generate_random_name()
        } else {
            typed
        };
        match save_layout(&name, &kdl, false) {
            Ok(()) => {
                self.remember_file(name, metadata);
                self.page = Page::List;
                self.finding = true;
                self.query.clear();
                self.notice = None;
            }
            Err(error) => self.notice = Some(error),
        }
    }

    fn paste(&mut self, text: String) -> bool {
        if !matches!(&self.page, Page::New(page) if page.kind == NewKind::Import) {
            return false;
        }
        self.apply_import_text(text);
        true
    }

    fn import_push(&mut self, ch: char) {
        let mut text = self.import_text();
        text.push(ch);
        self.apply_import_text(text);
    }

    fn import_backspace(&mut self) {
        let mut text = self.import_text();
        text.pop();
        self.apply_import_text(text);
    }

    fn import_text(&self) -> String {
        match &self.page {
            Page::New(page) => page.pasted.clone().unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn apply_import_text(&mut self, text: String) {
        let parsed = if text.is_empty() || !self.granted {
            None
        } else {
            Some(parse_layout(&text))
        };
        let Some(page) = self.page_new_mut() else {
            return;
        };
        if page.kind != NewKind::Import {
            return;
        }
        page.pasted = Some(text);
        match parsed {
            None => {
                page.import_meta = None;
                page.import_error = None;
                if !self.granted {
                    self.notice = Some(GRANT.to_string());
                }
            }
            Some(Ok(metadata)) => {
                page.import_meta = Some(metadata);
                page.import_error = None;
                self.notice = None;
            }
            Some(Err(error)) => {
                let (_, full) = explain(&error);
                page.import_meta = None;
                page.import_error = Some(full);
                self.notice = None;
            }
        }
    }

    fn remember_file(&mut self, name: String, metadata: LayoutMetadata) {
        let modified = metadata.update_time.parse().ok().or_else(unix_now_some);
        self.entries
            .retain(|entry| !(entry.origin == Origin::File && entry.name == name));
        self.entries.push(Entry {
            name: name.clone(),
            origin: Origin::File,
            modified,
            broken: false,
            error_short: String::new(),
            error_full: String::new(),
            metadata: Some(metadata.clone()),
            info: Some(LayoutInfo::File(name.clone(), metadata)),
        });
        sort_entries(&mut self.entries);
        self.query.clear();
        self.sync();
        if let Some(position) = self.found.iter().position(|found| {
            self.entries[found.index].name == name
                && self.entries[found.index].origin == Origin::File
        }) {
            self.selected = position;
        }
        self.scope_name = None;
        self.sync();
    }

    fn replace_catalog(&mut self, entries: Vec<Entry>) {
        let selected = self.selected_entry().map(|entry| entry.name.clone());
        self.entries = entries;
        self.sync();
        if let Some(name) = selected {
            if let Some(position) = self
                .found
                .iter()
                .position(|found| self.entries[found.index].name == name)
            {
                self.selected = position;
            }
        }
        self.scope_name = None;
        self.sync();
    }

    fn sync(&mut self) {
        self.found = filter(&self.entries, self.query.text());
        if self.found.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.found.len() {
            self.selected = self.found.len() - 1;
        }
        let name = self.selected_entry().map(|entry| entry.name.clone());
        if name != self.scope_name {
            self.scope = self
                .selected_entry()
                .map(scope_of)
                .unwrap_or(Scope::Current);
            self.scope_name = name;
        }
    }

    fn recover_missing_page(&mut self) {
        if matches!(self.page, Page::One(_)) && self.current_entry().is_none() {
            self.page = Page::List;
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.found
            .get(self.selected)
            .and_then(|found| self.entries.get(found.index))
    }

    fn current_entry(&self) -> Option<&Entry> {
        let Page::One(page) = &self.page else {
            return None;
        };
        self.entries.iter().find(|entry| {
            entry.name == page.name && entry.origin == page.origin && entry.broken == page.broken
        })
    }

    fn entry_mut(&mut self, name: &str, origin: Origin) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.name == name && entry.origin == origin)
    }

    fn page_one_mut(&mut self) -> Option<&mut OnePage> {
        match &mut self.page {
            Page::One(page) => Some(page),
            _ => None,
        }
    }

    fn page_new_mut(&mut self) -> Option<&mut NewPage> {
        match &mut self.page {
            Page::New(page) => Some(page),
            _ => None,
        }
    }

    fn page_new_ref(&self) -> Option<&NewPage> {
        match &self.page {
            Page::New(page) => Some(page),
            _ => None,
        }
    }

    fn list_notice(&self) -> Option<&str> {
        if let Some(notice) = self.notice.as_deref() {
            return Some(notice);
        }
        if self.granted {
            None
        } else {
            Some(GRANT)
        }
    }

    fn require_change(&mut self) -> bool {
        if self.granted {
            return true;
        }
        self.notice = Some(GRANT.to_string());
        false
    }

    fn require_read(&mut self) -> bool {
        self.require_change()
    }

    fn border_title(&self) -> String {
        match &self.page {
            Page::List if self.finding => "Layouts > Find".to_string(),
            Page::List => "Layouts".to_string(),
            Page::One(page) => format!("Layouts > {}", page.name),
            Page::New(_) => "Layouts > New".to_string(),
        }
    }

    fn retitle(&mut self) {
        if !self.granted {
            return;
        }
        let title = self.border_title();
        if self.border.as_deref() == Some(title.as_str()) {
            return;
        }
        rename_plugin_pane(get_plugin_ids().plugin_id, &title);
        self.border = Some(title);
    }

    fn hit_at(&self, y: usize, x: usize) -> Option<Click> {
        self.hits.iter().find_map(|hit| {
            if hit.y == y && x >= hit.x0 && x < hit.x1 {
                Some(hit.click)
            } else {
                None
            }
        })
    }

    fn paint_one(&self, rows: usize, cols: usize, pointer: Pointer) -> paint::Frame {
        let Some(entry) = self.current_entry() else {
            return paint::Frame {
                lines: vec![String::new(); rows],
                hits: Vec::new(),
                cursor: None,
            };
        };
        let Page::One(page) = &self.page else {
            return paint::Frame {
                lines: vec![String::new(); rows],
                hits: Vec::new(),
                cursor: None,
            };
        };
        let body = one_body(entry, page.debug, page.renaming, page.name_line.text());
        let cursor_x = if page.renaming {
            name_line(page.name_line.text(), true, page.name_line.cursor()).1
        } else {
            None
        };
        paint_page(
            self.notice.as_deref(),
            &body,
            cursor_x,
            &one_bars(entry.can_change_file(), entry.broken, page.renaming),
            &self.chrome,
            rows,
            cols,
            pointer,
        )
    }

    fn paint_new(&self, rows: usize, cols: usize, pointer: Pointer) -> paint::Frame {
        let Some(page) = self.page_new_ref() else {
            return paint::Frame {
                lines: vec![String::new(); rows],
                hits: Vec::new(),
                cursor: None,
            };
        };
        let session = page.kind == NewKind::Session;
        let mut body = Vec::new();
        let (name_text, cursor_x) =
            name_line(page.name_line.text(), page.editing, page.name_line.cursor());
        let can_save = if session {
            page.session_kdl.is_some() && page.session_meta.is_some()
        } else {
            page.import_meta.is_some() && page.import_error.is_none()
        };
        if can_save || page.editing {
            body.push(name_text);
            body.push(String::new());
        }
        if session {
            if let Some(metadata) = &page.session_meta {
                let entry = Entry {
                    name: "session".to_string(),
                    origin: Origin::File,
                    modified: None,
                    broken: false,
                    error_short: String::new(),
                    error_full: String::new(),
                    metadata: Some(metadata.clone()),
                    info: None,
                };
                body.extend(detail_lines(&entry));
            }
        } else {
            body.push("Layout:".to_string());
            match page.pasted.as_deref() {
                Some(text) if !text.is_empty() => {
                    body.extend(text.lines().map(str::to_string));
                }
                _ => body.push("Paste or type the layout here.".to_string()),
            }
            if let Some(error) = &page.import_error {
                body.push(String::new());
                body.extend(error.lines().map(str::to_string));
            } else if let Some(metadata) = &page.import_meta {
                body.push(String::new());
                let entry = Entry {
                    name: "import".to_string(),
                    origin: Origin::File,
                    modified: None,
                    broken: false,
                    error_short: String::new(),
                    error_full: String::new(),
                    metadata: Some(metadata.clone()),
                    info: None,
                };
                body.extend(detail_lines(&entry));
            }
        }
        let letters_are_keys = session;
        let mut bars = vec![new_choice_row(session, letters_are_keys)];
        if session && page.session_meta.is_some() {
            bars.push(new_tab_row(page.current_tab_only));
        }
        if page.editing {
            bars.push(vec![
                controls_key("ENTER", "Done", Click::Save),
                controls_key("ESC", "Cancel", Click::Back),
            ]);
        } else {
            bars.push(new_action_row(session, can_save, letters_are_keys));
        }
        paint_page(
            self.notice.as_deref(),
            &body,
            cursor_x,
            &bars,
            &self.chrome,
            rows,
            cols,
            pointer,
        )
    }
}

fn controls_key(key: &'static str, label: &'static str, click: Click) -> crate::controls::Piece {
    crate::controls::Piece::Key {
        key,
        label,
        short: label,
        click,
    }
}

fn is_apply(key: &KeyWithModifier) -> bool {
    matches!(key.bare_key, BareKey::Char('w') | BareKey::Char('W'))
        && key.key_modifiers.contains(&KeyModifier::Alt)
        && !key.key_modifiers.contains(&KeyModifier::Ctrl)
}

fn is_alt_right(key: &KeyWithModifier) -> bool {
    matches!(key.bare_key, BareKey::Right) && key.has_only_modifiers(&[KeyModifier::Alt])
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn unix_now_some() -> Option<i64> {
    Some(unix_now())
}

trait FormLine {
    fn handle_form(&mut self, key: &KeyWithModifier) -> Edit;
}

impl FormLine for Line {
    fn handle_form(&mut self, key: &KeyWithModifier) -> Edit {
        if key.has_no_modifiers() && matches!(key.bare_key, BareKey::Esc) {
            return Edit::Cancel;
        }
        if key.has_only_modifiers(&[KeyModifier::Ctrl])
            && matches!(key.bare_key, BareKey::Char('c'))
        {
            return Edit::Cancel;
        }
        self.handle(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line::key_with;

    #[test]
    fn finding_starts_on_the_builtin_list() {
        let state = State::default();
        assert!(state.finding);
        assert_eq!(state.entries[0].name, "default");
        assert_eq!(state.scope, Scope::Current);
        assert!(!state.keep_terminals);
        assert!(!state.keep_plugins);
        assert_eq!(state.border_title(), "Layouts > Find");
    }

    #[test]
    fn the_border_names_the_open_page() {
        let mut state = State::default();
        state.sync();
        state.open_info();
        assert_eq!(state.border_title(), "Layouts > default");
        state.page = Page::New(NewPage {
            kind: NewKind::Session,
            name_line: Line::new(),
            editing: false,
            current_tab_only: false,
            session_kdl: None,
            session_meta: None,
            pasted: None,
            import_meta: None,
            import_error: None,
        });
        assert_eq!(state.border_title(), "Layouts > New");
    }

    #[test]
    fn alt_w_is_apply() {
        let key = key_with(BareKey::Char('w'), &[KeyModifier::Alt]);
        assert!(is_apply(&key));
        assert!(!is_apply(&key_with(BareKey::Char('w'), &[])));
    }

    #[test]
    fn alt_right_opens_the_selected_layout_while_finding() {
        let mut state = State::default();
        state.sync();
        assert!(state.list_key(&key_with(BareKey::Right, &[KeyModifier::Alt])));
        match &state.page {
            Page::One(page) => assert_eq!(page.name, "default"),
            Page::List | Page::New(_) => panic!("expected the layout page"),
        }
    }
}
