---
version: 1
slug: "packages-desktop-src-ui-views-sql-editor-rs"
primary_target: "packages/desktop/src/ui/views/sql_editor.rs"
related_targets:
  - "packages/desktop/src/ui/views/result_grid.rs"
---

# SQL editor and result grid

Scope: the body of a SQL editor tab: toolbar, editor, result pane with the data grid and messages. The data grid specified here is reused by the table view. Mode: Operate. The target files do not exist yet.

Audience and job: an engineer writing ad-hoc SQL against a database they know, reading the answer, and adjusting. Success: write, run, and read without leaving the keyboard; large results stay fast; an error points at the place in the SQL; the environment is never in doubt.

World: Container Stencil, as in DESIGN.md (SQL Editor, Data Grid). The editor is the only ink surface.

## Layout

```
┌ toolbar 32 ────────────────────────────────────────────────────────────┐
│ [▶ Run] [Run selection] │ database ▾  schema ▾ │ [Auto-commit ▾]       │
├────────────────────────────────────────────────────────────────────────┤
│  1  SELECT id, name, created_at                                        │
│  2  FROM users                                    editor on ink        │
│  3  WHERE deleted_at IS NULL;                     14px padding         │
├──────────────────────── splitter (drag) ───────────────────────────────┤
│ RESULT 1   RESULT 2   MESSAGES                                         │ 24
│  #  │ ID   │ NAME │ CREATED_AT                                         │
│  1  │ 1042 │ ann  │ 2026-01-02 09:14:00                                │
│  2  │ 1043 │ bo   │ NULL                                               │
├────────────────────────────────────────────────────────────────────────┤
│ 120 rows · 34 ms · all rows fetched                      [Export CSV]  │ 24
└────────────────────────────────────────────────────────────────────────┘
```

The editor takes 40% of the height by default; the splitter moves between 15% and 85%. Before the first run the result pane is collapsed to its tab row and the editor takes the rest.

## Toolbar

- **Run** (Cmd/Ctrl+Enter): runs the statement under the cursor, or the selection if there is one. **Run all** (Cmd/Ctrl+Shift+Enter) runs every statement in order and stops at the first error.
- While running, Run becomes **Cancel** (Cmd/Ctrl+.) in outline style and the tab's state shape becomes the ring.
- **database / schema** selects (PostgreSQL): the session's current database and search path. SQLite shows neither.
- **Transaction mode:** Auto-commit (default) or Manual. In Manual, **Commit** (pending-confirm ochre) and **Rollback** (outline) appear once a statement has run, and the status line shows `TXN`. Closing a tab with an open transaction asks: Commit, Rollback, or Cancel.

## Editor

Body type on ink with a stencil-faint line-number gutter. Syntax colours per DESIGN.md: keywords stencil, identifiers and operators stencil-dim, numbers stencil, strings ochre-text, comments stencil-faint. Current line white/5, selection white/15. Completion (keywords, plus tables and columns already loaded in the navigator) opens as a rail popover with a hairline. IME and paste work as in any field.

Editor text is kept per tab and restored on restart. A statement that failed gets a 1px seal underline at the reported position, cleared on the next edit.

## Result pane

One tab per statement that returned rows (`RESULT n`), then `MESSAGES`. Tabs are label type; the active one is stencil with a 1px stencil underline.

### Data grid

- Header on rail in label type. Clicking a header sorts the fetched rows (ascending, descending, off) and shows a 10px arrow. The tooltip on a header gives the column's type.
- 24px rows, hairline rules, no striping. Fixed row-number column at the left in stencil-faint.
- Values in body-small. Numbers right-aligned with tabular figures. `NULL` as the label-style word in stencil-faint. Booleans as `true` / `false`. Long text truncates with an ellipsis; binary shows `<n bytes>`.
- Column width: sized to content on first load, between 64px and 320px; drag the header edge to resize; double-click it to fit.
- Selection: click a cell (1px stencil outline), Shift+click or Shift+arrows for a range, click a row number for the row. Copy puts tab-separated text on the clipboard; Copy as, in the context menu, offers CSV, JSON, and SQL INSERT.
- Enter or double-click on a cell opens a read-only value panel on rail at the right (320px) with the full value, wrapped; JSON is pretty-printed.
- Rows arrive 200 at a time; reaching the bottom fetches the next 200. Rendering is virtualised.
- Result grids are read-only. Editing happens in the table view.

### Footer (24px)

`<n> rows · <duration> · <fetched state>` where the fetched state is `all rows fetched` or `<n> fetched, more available`. Export CSV on the right exports the whole result, fetching the rest first, with a progress count and Cancel.

### Messages

A plain list in body-small, newest last: statement number, the first line of the SQL, then `n rows affected`, notices, or the error. An error entry has a seal 60% border, the driver's message verbatim, and `line n, column n` which moves the cursor there when clicked.

## States

- New tab: empty editor with a stencil-faint placeholder (`Write SQL. Cmd+Enter runs it.`), result pane collapsed.
- Running: ring on the tab, Cancel in the toolbar, footer shows elapsed time counting up. Previous results stay visible, dimmed to stencil-mute text.
- Rows returned: `RESULT n` tabs, the first one active.
- No rows: the grid shows its header and one stencil-faint line, `0 rows`.
- Statement without a result set: `MESSAGES` becomes active.
- Error: `MESSAGES` becomes active with the error entry; the editor underlines the position.
- Cancelled: a message entry `Cancelled after <duration>`; rows fetched so far stay.
- Connection held: the HELD panel from the shell replaces the result pane; the editor stays editable; Run reconnects first.
- Very wide or very long results: horizontal and vertical scroll inside the grid only; the header and the row-number column stay fixed.

## Keyboard

| Action | Shortcut |
|---|---|
| Run statement or selection | Cmd/Ctrl+Enter |
| Run all | Cmd/Ctrl+Shift+Enter |
| Cancel | Cmd/Ctrl+. |
| Commit / Rollback (manual mode) | Cmd/Ctrl+Shift+C / Cmd/Ctrl+Shift+R |
| Move focus editor ↔ results | F6 |
| Next / previous result tab | Alt+Right / Alt+Left |
| Grid: move | Arrows, Home / End, Page Up / Page Down |
| Grid: extend selection | Shift + the same |
| Grid: copy | Cmd/Ctrl+C |
| Grid: open value panel | Enter |
| Grid: select all | Cmd/Ctrl+A |
| Export CSV | Cmd/Ctrl+E |

## Components

GPUI Kit: `input` code editor (`input/editor.rs`) with the `highlighter` module for SQL, `table` for the grid (virtualised), `resizable` for the split, `tab` for result tabs, `select`, `button`, `menu`, `tooltip`, `spinner`. The grid lives in its own file so the table view can reuse it.

## As built

Where the first version differs from the text above:

- Run runs the selection, or the whole editor when nothing is selected. There is no statement-under-cursor and no separate Run all.
- No database or schema selects, no transaction mode, no completion, and editor text is not kept across restarts.
- A query is read whole, up to 10,000 rows, then cut short; rows are not fetched on scroll.
- The grid selects one cell, not a range. Copy cell and Copy row are in the context menu; there is no value panel, no Copy as, and no CSV export.
- Column headers show the names verbatim rather than in tracked caps, because names are case-sensitive.
- On PostgreSQL a failed query reports the error alone, numeric columns are detected from their values, and every result of one run carries the time of the whole run.
- Highlighting uses tree-sitter (`tree-sitter-sql` feature of the kit).

## Open decisions

- SQL highlighting needs the kit's tree-sitter SQL grammar or its syntect backend; pick one at build time.
- The default row limit is unbounded with paged fetch. A hard `LIMIT` setting is left for later.
- Saved queries, query formatting, and EXPLAIN visualisation are not in the first version.
