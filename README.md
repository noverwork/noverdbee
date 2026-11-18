<div align="center">

# Noverdbee

**Databases for you and your AI.**

An open-source PostgreSQL and SQLite client with a built-in MCP server, so you and your AI agent work the same databases from one app. Native, built with GPUI and Rust, for macOS, Windows, and Linux.

[![Release](https://img.shields.io/github/v/release/noverwork/noverdbee)](https://github.com/noverwork/noverdbee/releases)
[![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20Windows%20%7C%20Linux-blue)]()
[![License](https://img.shields.io/badge/License-MIT-5f3dc4)](LICENSE)

[Download](https://github.com/noverwork/noverdbee/releases) · [Website](https://noverwork.github.io/noverdbee/) · [Report a bug](https://github.com/noverwork/noverdbee/issues)

</div>

![Noverdbee with a production query tab: a SQL join in the editor, three result rows below, and the connection navigator on the left](docs/assets/shots/editor-results.webp)

## Highlights

- **MCP server for AI agents** — Claude Code, Cursor, VS Code, and Claude Desktop can list tables and run select, insert, update, or delete on the connections where you allow each tool
- **Environment-coloured tabs** — production, staging, development, and local each have their own colour and code, so you know where a query runs before you run it
- **SQL editor and result grid** — run a selection or the whole script, page through large results, copy and paste cell blocks
- **Staged row edits** — edits, new rows, and deletions stay marked until you review the SQL and save
- **Table structure** — columns, indexes, foreign keys, and the definition
- **Local-first** — no account, no cloud sync, no telemetry

Work in progress. It connects to SQLite files and PostgreSQL servers, browses tables, runs SQL, and edits rows with staged changes. The design is in `PRODUCT.md`, `DESIGN.md`, and `.impeccable/surfaces/`. Saved connections, including passwords in plain text, stay in a local SQLite database.

## Download

| Platform | Package |
|----------|---------|
| macOS (Apple Silicon) | `.dmg` |
| Windows | `.exe` installer |
| Linux (x86-64) | `.AppImage` or `.deb` |

Get the latest version from [Releases](https://github.com/noverwork/noverdbee/releases). Noverdbee checks for updates on its own after install.

On macOS, install from the terminal (builds are not notarized, so this avoids the Gatekeeper prompt):

```bash
curl -fsSL https://raw.githubusercontent.com/noverwork/noverdbee/main/install.sh | sh
```

If you install the `.dmg` instead, macOS blocks the first launch. Open **System Settings → Privacy & Security** and click **Open Anyway**, or run `xattr -dr com.apple.quarantine /Applications/noverdbee.app`.

## Development

```sh
cargo make dev        # run the app
cargo make ci         # clippy, fmt check, tests
cargo make screenshots  # render the pages offscreen to target/screenshots
```

The PostgreSQL test needs a server and is skipped without one:

```sh
docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:17
NOVERDBEE_TEST_PG_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test -p desktop postgres
```

## License

MIT
