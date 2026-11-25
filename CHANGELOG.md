# Changelog

## 1.0.0 (2026-11-25)


### Features

* add a window for opening, applying, and recording layouts ([7ff24ee](https://github.com/fulldecent/zellij-layout-manager-ng/commit/7ff24eeacd72d8d33137d6528fa2a42ffd7233e9))


### Miscellaneous Chores

* count unreleased layout work toward 1.0.0 ([4b15ea4](https://github.com/fulldecent/zellij-layout-manager-ng/commit/4b15ea4cdb059d21b01db3326a7a52a3c5afd906))

## Changelog

## Unreleased

- The list shows layout files and the built-in layouts `default`, `compact`, `classic`, `strider`, and `disable-status-bar`.
- Enter opens the selection as new tabs. `⌥W` applies it to the session. Apply To Tab chooses the current tab or every tab. Unmatched Terminals and Unmatched Plugins start on Close.
- Import keeps the pasted or typed layout on the page and parses it.
- Key names on the bar use ENTER, ESC, TAB, and DEL.
- A layout page shows its tabs and panes. A file can be edited, renamed, or deleted. A file that failed to parse shows the diagnostic.
- New records the current session or imports pasted KDL.
