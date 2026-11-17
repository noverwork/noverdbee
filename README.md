# Noverdbee

A desktop database client, built with GPUI and Rust. PostgreSQL and SQLite first.

Work in progress. It connects to SQLite files and PostgreSQL servers, browses tables, runs SQL, and edits rows with staged changes. The design is in `PRODUCT.md`, `DESIGN.md`, and `.impeccable/surfaces/`. Saved connections, including passwords in plain text, stay in a local SQLite database.

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
