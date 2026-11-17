# Noverdbee

A desktop database client, built with GPUI and Rust. PostgreSQL and SQLite first.

Work in progress: the database side is not built yet. What is here so far is the app shell, settings with backup and restore, and the updater. The design is in `PRODUCT.md`, `DESIGN.md`, and `.impeccable/surfaces/`. All data stays in a local SQLite database.

## Development

```sh
cargo make dev        # run the app
cargo make ci         # clippy, fmt check, tests
```

## License

MIT
