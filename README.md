# Noverdbee

A desktop database client, built with GPUI and Rust. PostgreSQL and SQLite first.

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
