---
version: 1
slug: "packages-desktop-src-ui-views-db-connection-form-rs"
primary_target: "packages/desktop/src/ui/views/db_connection_form.rs"
related_targets: []
---

# Database connection form

Scope: the full-page form that adds or edits a saved database connection, including Test connection. Mode: Operate. The target file does not exist yet.

Audience and job: an engineer adding a database they already have the address and credentials for. Success: the form asks only for what the chosen driver needs, the connection can be tested before saving, and a failure shows the driver's own error.

World: Container Stencil, as in DESIGN.md. Nothing new is introduced here; this surface reuses the existing form page.

## Layout

A full-page route built with `common::form_page`: headline, hairline rule, sections, button row. The navigator is hidden and the tab strip dims to 45% while it is open. Content column is 640px wide at most; fields are 44px.

```
New database connection
────────────────────────────────────────────────
CONNECTION
  Driver      [ PostgreSQL ▾ ]
  Name        [ orders                    ]
  Group       [ Production ▾ ]            ▮ PRD·ORDERS

SERVER                                (PostgreSQL only)
  Host        [ 10.0.4.12        ]  Port [ 5432 ]
  Database    [ orders                    ]
  User        [ app                       ]
  Password    [ ••••••••               👁 ]
  SSL mode    [ prefer ▾ ]

FILE                                  (SQLite only)
  Path        [ /Users/me/data/app.db ] [ Browse ]
  Read-only   ( ○ )

  ┌ Connected · PostgreSQL 16.3 · 41 ms ────────┐   ← test result
  └─────────────────────────────────────────────┘
[ Test connection ]              [ Cancel ] [ Save ]
```

## Fields

- **Driver:** PostgreSQL or SQLite. Changing it swaps the sections below and keeps Name and Group. It is locked when editing a saved connection.
- **Name:** required, unique within its group.
- **Group:** existing groups plus None. Beside it, a live preview of the swatch and `GRP·CONN` code the tabs will carry.
- **PostgreSQL:** Host (required), Port (default 5432, digits only), Database (default `postgres`), User (required), Password (masked with a reveal control; optional), SSL mode (`disable`, `prefer`, `require`, `verify-full`; default `prefer`).
- **SQLite:** Path (required; Browse opens the system file dialog; the path may be typed), Read-only toggle. A path that does not exist shows an ochre note that the file will be created on first connect, unless Read-only is on, which makes it an error.

## Test connection

Test connection runs with the current field values without saving. While it runs the button shows a spinner and reads Testing; Save stays enabled. The result appears in a panel above the button row and stays until a field changes.

- Success: hairline panel, `Connected · <server version> · <duration>`.
- Failure: seal 60% border, the driver's error text verbatim in body-small, selectable.

## States

- New: empty fields with defaults; Save disabled until required fields are filled.
- Edit: fields filled; the password field shows a placeholder meaning "unchanged" and is only rewritten when typed into.
- Validation: on blur and on Save. An invalid field takes the destructive edge and a body-small message below it. The first invalid field takes focus on Save.
- Saving: Save shows a spinner; on success return to the Databases route with the new connection selected in the navigator.
- Save failed: `error_banner` above the button row.

## Keyboard

Tab order follows the visual order. Enter in a field does nothing; Cmd/Ctrl+Enter saves; Cmd/Ctrl+Shift+Enter tests; Esc cancels (asks first only if a field changed).

## Components

Reuse from `packages/desktop/src/ui/views/common.rs`: `form_page`, `form_section`, `field`, `form_buttons`, `error_banner`, `Choice` with `new_select` and `selected`. GPUI Kit: `input`, `select`, `switch`, `button`, `spinner`. Group preview uses `stencil::line_color` and `stencil::group_prefix`.

## As built

Where the first version differs from the text above:

- The driver can be changed while editing a saved connection.
- The group is a text field, with no live preview of the swatch and code.
- Fields are checked when Test connection or Save is pressed, not on blur. Cmd/Ctrl+Enter and Esc are not bound.
- SSL modes are `disable`, `prefer` and `require`; certificates are checked against the system roots only.
- The password is saved in plain text in the app's SQLite store and is shown again when editing.

## Open decisions

- Where the password is stored (the app's SQLite store in plain text, or the OS keychain). The form is the same either way.
- A connection-string paste shortcut (`postgres://…`) and SSH tunnels are left for later.
