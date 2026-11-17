---
version: 1
slug: "packages-desktop-src-ui-shell-rs"
primary_target: "packages/desktop/src/ui/shell.rs"
related_targets: []
---

# App shell

Scope: the persistent shell around every view: left rail, navigator panel (object tree and history), top bar (page label + workspace tabs), the workspace area, and a bottom status line. Mode: Operate.

Audience and job: backend/DevOps engineers holding connections to several databases across environments for hours, keyboard first. Success: the active connection and its environment read at a glance before any statement runs; dropped connections and open transactions are unmistakable; nothing moves from where the user's hands expect it (tabs stay on top, drag-reorder, right-click menu, rail order).

Constraints (user-pinned): dark; Sarasa Term mono only; must not read as a VS Code/IDE clone; must not be bland grey system UI; left tree + tabbed workspace layout; PostgreSQL and SQLite in the first version.

## Direction contract

THESIS: Every connection is a container: identity stencilled as a group code, carried on its group's line colour, sealed when held. Every tab opened from it is a door of that container. Refuses the category default of grey tabs over a grey tree where production and local look the same.

OWN-WORLD: Night-port slate ground (#101317 / #0b0e11 rail), corrugated tab strip (1px vertical groove on #1a1f25), white stencil type (#e8e4d8) tracked 0.14em, line colours rust #a33b28, harbour blue #2a648f, evergreen #336a41, ochre #c9952e, graphite #2a2f36 for ungrouped. Whole-pixel 1px hairlines only; no blur, no glass, no radial glows. Square corners (2px max).

STORY: The engineer sees which environment each tab belongs to by colour and code before reading the query, sees the active one outlined in stencil white, sees a dropped one sealed HELD, and sees `TXN` in the status line while a transaction is open.

FIRST VIEWPORT:

```
┌────┬────────────────┬──────────────────────────────────────────────┐
│logo│ OBJECTS HISTORY│ [PRD·orders users ●][DEV·app Query 2 ○]   +  │ 36 top bar
│    │ [filter      ] ├──────────────────────────────────────────────┤
│ DB │ ▮ PRODUCTION   │                                              │
│    │   PRD  2       │                                              │
│    │  ● orders      │              workspace                       │
│    │   ▾ public     │   (SQL editor tab or table tab)              │
│    │     ▸ users    │                                              │
│    │     ▸ invoices │                                              │
│    │  HELD billing  │                                              │
│    │ ▮ DEVELOPMENT  │                                              │
│ SET│  ○ app         │                                              │
├────┴────────────────┴──────────────────────────────────────────────┤
│ ▮ PRD·ORDERS  app@10.0.4.12:5432/orders  Connected  TXN  120 rows · 34 ms      3 open · 1 held │ 24
└────────────────────────────────────────────────────────────────────┘
 56        240 (200–400)
```

FORM: Container Stencil, position 6 of 7 on the grounded list (re-roll round 1). seed:0e673c982a0ba5b4
Raises: Pixel honesty (from Emigre): every rule a 1px whole-pixel hairline, nothing soft. One shape per state (from Labanotation): filled dot live, ring waiting, seal dropped, readable without colour. Fixed anatomy (from Botanical folio): every tab identical anatomy and width rules; only state changes.
Signature move: line colour + stencil group code on every workspace tab and the status line, derived from the connection's group.
Motion grammar: none decorative; 120ms opacity/outline transitions on tab state; the HELD seal appears without animation; reduced motion removes transitions.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Regions

### Rail (56px)
Logo cell, then one icon item, Databases (opens the workspace), and Settings at the foot. Databases is the home route. Settings and forms are full-page routes.

### Navigator (240px, drag 200–400px, collapsible)
Shown only on the Databases route. On rail, separated from the workspace by a rib rule that is also the drag handle.

- Header: two label-style switches, OBJECTS and HISTORY (the active one in stencil with a 1px stencil underline), then a 24px filter field, then a `+` icon button for New connection.
- OBJECTS tree:
  - Group heading: swatch, group name, code chip, count chip. Ungrouped connections sit under no heading, first.
  - Connection row: state shape, name. Opening a row connects.
  - PostgreSQL: database → schema → `Tables`, `Views`, `Functions` folders → object → columns.
  - SQLite: `Tables`, `Views`, `Indexes` folders → object → columns.
  - Column row: name, then type right-aligned in stencil-faint; a key icon marks primary-key columns.
  - Children load when a node first opens; a 12px spinner replaces the chevron while loading.
- The filter narrows the loaded tree by name and keeps the parents of matches.
- HISTORY list: newest first, one 40px row each: code chip, first line of the SQL (truncated), then duration and a seal-tinted `FAILED` label when it failed. Opening a row puts the SQL in a new editor tab on the same connection.

Context menus:
- Connection: Connect / Disconnect, New SQL editor, Refresh, Edit, Duplicate, Delete.
- Table or view: Open data, Open structure, New SQL editor, Copy name, Refresh.
- Group heading: New connection in group, Rename, Delete (asks first; connections move to ungrouped).

### Top bar (36px)
Page-label cell on rail (the label of the current route), then the corrugated strip of workspace tabs, then a `+` button that opens a new SQL editor on the active tab's connection.

Tab anatomy, left to right: `GRP·CONN` code, title, state shape, close control. Titles: `Query n` for editors, the table name for data tabs, `table · structure` for structure tabs. A tab with unsaved staged edits shows a 6px ochre square before the title. Tabs reorder by drag. Tab context menu: Close, Close others, Close all on this connection, Duplicate (editors only), Reconnect (when held).

### Status line (24px)
Left to right: swatch + active code; address (`user@host:port/database`, or the file path for SQLite); state label; `TXN` while a transaction is open; last statement's rows and duration. Right: `n open · n held`. With no tab open it shows only the counts.

## States

- First run, no connections: the navigator shows a short empty state ("No connections yet") with a New connection button; the workspace shows the same action and nothing else. The tab strip is empty.
- Connections exist, no tab open: the workspace shows the three most recently used connections as rows and the shortcut for New SQL editor.
- Connecting: ring on the connection row and its tabs; tree children wait.
- Connect failed or connection dropped: HELD seal on the row and its tabs; the tab body shows a seal-bordered panel with the driver's error text and a Reconnect button. Editor text and staged edits are kept.
- Tree load error: a single row under the node with the error in ochre-text and a Retry action.
- Narrow window (under 900px): the navigator collapses; the rail item toggles it as an overlay on rail.
- Backend unavailable: the existing error screen.

## Keyboard

| Action | Shortcut |
|---|---|
| Toggle navigator | Cmd/Ctrl+B |
| Focus navigator filter | Cmd/Ctrl+Shift+F |
| New SQL editor | Cmd/Ctrl+T |
| Close tab | Cmd/Ctrl+W |
| Next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab |
| Go to tab 1–9 | Cmd/Ctrl+1–9 |
| New connection | Cmd/Ctrl+N |
| Settings | Cmd/Ctrl+, (existing) |
| Tree: move, open, close | Up / Down, Right or Enter, Left |
| Tree: open data for a table | Enter |
| Tree: context menu | Shift+F10 or the menu key |

## Components

GPUI Kit modules: `tree` (navigator), `tab` (workspace strip), `resizable` (navigator and editor splits), `status_bar`, `menu`, `input` (filter), `spinner`, `tooltip`. Reuse `stencil::line_color` and `stencil::group_prefix` for swatches and codes, `icons::lucide`, `theme` constants, and `i18n::t` for all copy.

## Recorded adaptations
- Line colours are darkened to rust #a33b28, harbour blue #2a648f, evergreen #336a41 so stencil text keeps >= 4.5:1; ochre #c9952e takes dark text when filled. No fifth colour.
- Inactive tabs mix their line colour 55% into the rib (instead of 55% opacity) so the stencil text keeps contrast; they carry a 1px white/10 edge so ungrouped tabs still read as containers.
- The rail keeps Lucide icons instead of the sketch's text codes.
- Pixel honesty applies app-wide: no soft shadows, 2px corners, editor ink is pure black, secondary text is stencil-derived (#c1beb5 / #9f9d96 / #82807b).

## Unresolved
- Groups have no stored colour; line colour is derived deterministically from the group. User-assigned colours are a later product decision.
- Saved queries, a command palette, and split editors are not designed yet.
