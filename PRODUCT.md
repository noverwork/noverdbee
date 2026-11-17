# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

Desktop app for macOS (Apple Silicon), Windows, and Linux, drawn with GPUI. The design language is the app's own, not native AppKit/WinUI.

## Users

Backend and DevOps engineers who work against several databases every day. They keep connections to production, staging, and local databases open for hours, browse schemas, run ad-hoc SQL, and inspect or fix rows while debugging or operating a system.

## Product Purpose

Noverdbee is a database management tool: saved connections, an object navigator, a tabbed SQL editor, a result grid, and a table data and structure viewer. Success means an engineer can do their whole day of database work from one fast window, always knowing which environment a query is about to run against, without the client itself costing noticeable memory or attention.

## Positioning

Native and lightweight: built on GPUI and Rust, with no Electron, no webview, and no JVM. Speed and low overhead are the core claim; the UI should never undermine it.

## Operating Context

- Long-lived connections: several databases open at once across environments, many editor and table tabs per connection, and dropped connections that reconnect when the user returns to the tab.
- Keyboard-heavy use: write SQL, run it, read the result, and move between tabs without the mouse. IME input and paste must work in the editor and in grid cells.
- Results range from one row to millions; rows are fetched in pages, never all at once.
- All app data (connections, groups, query history, settings) lives in a local SQLite database in the app data folder; no cloud account or sync.

## Capabilities and Constraints

- Built so far: the app shell, connection groups, settings with backup and restore, auto-update from GitHub Releases.
- Planned for the first version: PostgreSQL and SQLite connections, object navigator, tabbed SQL editor, result grid, table data viewer and editor, table structure viewer, query history.
- Later, not in the first version: MySQL and SQL Server, SSH tunnels, saved queries, structure editing, import, ER diagrams, command palette.
- Stack: GPUI with GPUI Kit components in the UI; Rust, Tokio, Diesel/SQLite behind it.
- UI copy ships in English and Traditional Chinese (zh-TW).
- Open source under the MIT License; pull requests are welcome.

## Brand Commitments

- Name: Noverdbee (by noverwork).
- Voice: direct, slightly dry developer humor.
- Existing assets: app icons in `packages/desktop/icons/`.

## Evidence on Hand

- No memory or speed benchmark has been published yet.
- No screenshots, testimonials, or user counts exist. Do not invent them.

## Product Principles

1. The data is the product. The editor and the grid get the space; chrome around them stays out of the way.
2. Always know where you are. The environment a tab belongs to reads before the query does, so nobody runs a statement against production by mistake.
3. Never feel heavier than the claim. Interactions must be instant; no decorative motion or weight that contradicts "lightweight".
4. Keyboard first. Every frequent action is reachable without the mouse.
5. Nothing changes silently. Edits to rows are staged and shown before they are written; open transactions and dropped connections are always visible.
6. Local and trustworthy. Make credentials and connection state legible.
