---
version: 1
slug: "packages-desktop-src-ui-views-table-view-rs"
primary_target: "packages/desktop/src/ui/views/table_view.rs"
related_targets:
  - "packages/desktop/src/ui/views/result_grid.rs"
---

# Table view (data and structure)

Scope: the body of a table tab opened from the navigator. A data tab browses and edits rows; a structure tab shows columns, indexes, foreign keys, and DDL. Mode: Operate. The target file does not exist yet.

Audience and job: an engineer checking what is in a table, fixing a few rows, or confirming how a table is defined. Success: rows are found with a filter rather than hand-written SQL; no edit reaches the database until the engineer has seen it and saved it.

World: Container Stencil, as in DESIGN.md (Data Grid, The Ochre Is Pending Rule). The grid is the one specified in the SQL editor surface, with editing added.

## Data tab

```
┌ toolbar 32 ────────────────────────────────────────────────────────────┐
│ WHERE [ deleted_at IS NULL                    ] [Apply] │ [↻] [+ Row]  │
├────────────────────────────────────────────────────────────────────────┤
│  #  │ ID ⚷ │ NAME   │ EMAIL            │ DELETED_AT                    │
│  1  │ 1042 │ ann    │ ann@example.com  │ NULL                          │
│  2  │ 1043 │▕bo    ▏│ bo@example.com   │ NULL            ← edited cell │
│  3  │ 1044 │ cy     │ cy@example.com   │ NULL            ← staged delete│
├────────────────────────────────────────────────────────────────────────┤
│ 3 pending changes   [Review SQL] [Discard] [Save]   200 of ~18,400 rows│ 24
└────────────────────────────────────────────────────────────────────────┘
```

### Toolbar

- **WHERE** field (24px, inline): a SQL condition applied to the table. Enter or Apply runs it; an empty field shows all rows. An invalid condition takes the destructive edge and shows the driver's error below the toolbar.
- **Refresh** reloads with the current filter and sort. **+ Row** appends an empty staged row.
- Sorting by header click re-queries with `ORDER BY` here (unlike a result grid, which sorts fetched rows only).

### Editing

- Enter, F2, or double-click on a cell starts editing in place; Enter commits to the staging area, Esc reverts the cell. Typing over a selected cell starts editing with that text.
- Set NULL (Cmd/Ctrl+Backspace) is separate from an empty string.
- Delete on selected rows stages them for deletion.
- Staged changes are not sent until Save. An edited cell shows ochre-text with a 1px ochre edge; a new row has an ochre edge on the whole row; a row staged for deletion has a seal edge and stencil-faint text. The tab shows the 6px ochre square before its title.
- The footer shows `<n> pending changes` with **Review SQL** (opens a rail dialog listing the exact statements), **Discard** (outline), and **Save** (pending-confirm ochre). Save runs all statements in one transaction; if any fails, none apply, the failing row is scrolled into view, and the driver's error appears in a seal-bordered panel above the footer.
- A table without a primary key (or, for SQLite, without a usable rowid) is read-only: editing controls are absent and the footer carries a stencil-mute note, `Read-only: no primary key`. Views are read-only the same way.
- Refresh, a new filter, or closing the tab with pending changes asks: Save, Discard, or Cancel.

### Footer

Pending-change controls on the left when any exist. On the right, `<fetched> of ~<estimate> rows` (the estimate comes from the database's statistics; `of <n>` without the tilde once every row is fetched) and the last load's duration.

## Structure tab

Sub-tabs in label type under the tab strip: `COLUMNS`, `INDEXES`, `FOREIGN KEYS`, `DDL`. All read-only in the first version.

- **Columns:** a grid of position, name, type, nullable, default, key (primary-key marker), comment.
- **Indexes:** name, columns, unique, method (PostgreSQL), definition.
- **Foreign keys:** name, columns, referenced table and columns, on update, on delete. The referenced table is a link that opens its structure tab.
- **DDL:** the `CREATE TABLE` statement on ink in the editor's read-only mode with the same syntax colours and a Copy button. For SQLite this is the stored `sql` from `sqlite_master`; for PostgreSQL it is reconstructed from the catalog and marked as such in a body-small note.

## States

- Loading: the grid header appears as soon as columns are known; a 12px spinner sits in the footer until the first rows arrive.
- Empty table: header plus one stencil-faint line, `0 rows`, and + Row still available.
- Filter matches nothing: `0 rows match` with a Clear filter action.
- Load error: seal-bordered panel in place of the grid with the driver's error and Retry.
- Save in progress: Save shows a spinner; the grid is not editable until it returns.
- Table changed or dropped since the tab opened: the load error panel, with the navigator's Refresh suggested.
- Connection held: the HELD panel from the shell; staged edits are kept and reapplied to the view after reconnect.
- No indexes or no foreign keys: an `empty_state` line in that sub-tab.

## Keyboard

Grid movement, selection, copy, and the value panel work as in the SQL editor surface. In addition:

| Action | Shortcut |
|---|---|
| Edit cell | Enter or F2 |
| Commit / revert cell edit | Enter / Esc |
| Set NULL | Cmd/Ctrl+Backspace |
| Add row | Cmd/Ctrl+I |
| Stage delete for selected rows | Delete |
| Save pending changes | Cmd/Ctrl+S |
| Discard pending changes | Cmd/Ctrl+Shift+Z |
| Focus the WHERE field | Cmd/Ctrl+F |
| Refresh | Cmd/Ctrl+R |
| Switch data ↔ structure for this table | Cmd/Ctrl+D |

## Components

The shared grid from `result_grid.rs` (GPUI Kit `table`), plus `input` for in-cell editing and the WHERE field, `tab` for structure sub-tabs, `dialog` for Review SQL and the unsaved-changes prompt, the kit's code editor in read-only mode for DDL, `button`, `spinner`. Reuse `common::empty_state`, `common::error_banner`, and `common::confirm_delete`'s dialog pattern.

## Open decisions

- Editors for specific types (date picker, JSON editor, enum select) are left for later; the first version edits every value as text and lets the database reject bad input.
- Structure editing (add column, create index) is not in the first version.
- Column filters from the header (as opposed to the WHERE field) are left for later.
