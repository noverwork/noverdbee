//! Renders the real window offscreen and saves PNGs, so a change to the UI can
//! be looked at without a display. Needs a GPU: Metal on macOS, WGPU on Linux.
//!
//! `cargo test -p desktop --test screenshots` writes to `target/screenshots/`.

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("screenshots: skipped; no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use desktop_lib::ui::{self, store::Route};
    use gpui_kit::{
        px, size, AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions,
    };

    struct Stage {
        cx: HeadlessAppContext,
        window: AnyWindowHandle,
        out: PathBuf,
    }

    impl Stage {
        /// Lets backend work finish and its results reach the views.
        fn settle(&mut self, duration: Duration) {
            let steps = (duration.as_millis() / 50).max(1);
            for _ in 0..steps {
                std::thread::sleep(Duration::from_millis(50));
                self.cx.run_until_parked();
            }
        }

        fn shoot(&mut self, name: &str) {
            self.cx
                .update_window(self.window, |_, window, cx| {
                    window.refresh();
                    let _ = window.draw(cx);
                })
                .ok();
            self.cx.run_until_parked();
            let image = self
                .cx
                .capture_screenshot(self.window)
                .expect("offscreen rendering must be available");
            let path = self.out.join(format!("{name}.png"));
            image.save(&path).expect("save screenshot");
            println!("screenshot: {}", path.display());
        }

        fn close_dialogs(&mut self) {
            use gpui_kit::component::WindowExt as _;
            self.cx
                .update_window(self.window, |_, window, cx| window.close_all_dialogs(cx))
                .ok();
            self.settle(Duration::from_millis(400));
        }

        fn store<R>(
            &mut self,
            update: impl FnOnce(&mut ui::store::Store, &mut gpui_kit::Context<ui::store::Store>) -> R,
        ) -> R {
            self.cx.update(|cx| ui::store::store(cx).update(cx, update))
        }
    }

    /// A SQLite database to browse, and saved connections in and out of groups.
    /// Returns the id of the connection to the database.
    fn seed(app: &desktop_lib::app::AppHandle, directory: &std::path::Path) -> String {
        use desktop_lib::db::{Session, DRIVER_SQLITE};
        use desktop_lib::store::connections::connection_save;
        use shared::DbConnectionRecord;

        let path = directory.join("shop.db").to_string_lossy().into_owned();
        let sqlite = |name: &str, path: &str| DbConnectionRecord {
            name: name.to_string(),
            driver: DRIVER_SQLITE.to_string(),
            path: path.to_string(),
            ..Default::default()
        };
        desktop_lib::app::runtime().block_on(async {
            let session = Session::open(&sqlite("seed", &path)).await.expect("seed db");
            let results = session
                .run(
                    "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT, created_at TEXT NOT NULL);
                     CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers (id), total NUMERIC NOT NULL, note TEXT);
                     CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 100;
                     INSERT INTO customers (name, email, created_at) VALUES
                       ('Ann Lee', 'ann@example.com', '2026-01-02 09:14:00'),
                       ('Bo Chen', NULL, '2026-02-11 17:40:12'),
                       ('O''Brien & Sons', 'hello@obrien.example', '2026-03-05 08:00:00');
                     INSERT INTO orders (customer_id, total, note) VALUES (1, 42.5, NULL), (1, 310, 'gift wrap'), (3, 1280.75, NULL);"
                        .to_string(),
                )
                .await;
            assert_eq!(results.len(), 5, "seed statements ran: {results:?}");

            let mut shop = String::new();
            use shared::Environment;
            for (name, file, group, environment) in [
                ("shop", path.as_str(), "Shop", Environment::Production),
                ("shop-replica", "/var/data/replica.db", "Shop", Environment::Staging),
                ("staging", "/var/data/staging.db", "Shop", Environment::Staging),
                ("scratch", "/tmp/scratch.db", "", Environment::Local),
            ] {
                let record = DbConnectionRecord {
                    environment,
                    ..sqlite(name, file)
                };
                let saved = connection_save(record, group.to_string(), app.state())
                    .await
                    .expect("save connection");
                if name == "shop" {
                    shop = saved.id;
                }
            }
            use desktop_lib::store::tunnels;
            let bastion = tunnels::ssh_host_save(
                shared::SshHostRecord {
                    name: "bastion".to_string(),
                    host: "bastion.internal".to_string(),
                    port: 22,
                    username: "ubuntu".to_string(),
                    password: "secret".to_string(),
                    ..Default::default()
                },
                app.state(),
            )
            .await
            .expect("save SSH host");
            tunnels::kubeconfig_save(
                shared::KubeconfigRecord {
                    name: "staging".to_string(),
                    config: "apiVersion: v1\nkind: Config\ncurrent-context: staging\n".to_string(),
                    ..Default::default()
                },
                app.state(),
            )
            .await
            .expect("save kubeconfig");
            let warehouse = DbConnectionRecord {
                name: "warehouse".to_string(),
                driver: desktop_lib::db::DRIVER_POSTGRES.to_string(),
                host: "db.internal".to_string(),
                port: 5432,
                database: "warehouse".to_string(),
                username: "analyst".to_string(),
                password: "secret".to_string(),
                ssl_mode: "require".to_string(),
                tunnel: desktop_lib::db::tunnel::TUNNEL_SSH.to_string(),
                ssh_host_id: Some(bastion.id),
                ..Default::default()
            };
            connection_save(warehouse, "Analytics".to_string(), app.state())
                .await
                .expect("save connection");
            shop
        })
    }

    /// Opens the main window offscreen at `width` x `height` points.
    fn open_stage(
        app: &desktop_lib::app::AppHandle,
        out: PathBuf,
        width: f32,
        height: f32,
    ) -> Stage {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(ui::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.allow_parking();
        let window = cx
            .update(|cx| {
                ui::init(app.clone(), cx);
                // Dialogs would otherwise be caught mid-entrance.
                cx.set_reduce_motion(true);
                ui::open_main_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Default::default(),
                            size: size(px(width), px(height)),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                )
            })
            .expect("window opens");
        let mut stage = Stage { cx, window, out };
        stage.settle(Duration::from_millis(500));
        stage
    }

    pub fn run() {
        let data = tempfile::tempdir().expect("scratch data dir");
        std::env::set_var("NOVERDBEE_DATA_DIR", data.path());
        // Screenshots must not depend on what GitHub answers.
        std::env::set_var("NOVERDBEE_SKIP_UPDATE_CHECK", "1");
        let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        std::fs::create_dir_all(&out).expect("screenshot dir");

        let app = desktop_lib::bootstrap::backend().expect("backend starts");
        let shop = seed(&app, data.path());
        // The language otherwise follows the system; the website wants `en`.
        if let Ok(code) = std::env::var("NOVERDBEE_SCREENSHOT_LOCALE") {
            let locale = shared::Setting {
                key: ui::i18n::LOCALE_SETTING.to_string(),
                value: code,
            };
            desktop_lib::app::runtime()
                .block_on(desktop_lib::store::settings::set_setting(
                    locale,
                    app.state(),
                ))
                .expect("save locale");
        }
        let mut stage = open_stage(&app, out, 1200., 800.);

        stage.shoot("databases");

        // Open one connection and fail another, to see both states.
        let failing = stage.store(|store, _| {
            let staging = store.connections.iter().find(|row| row.name == "staging");
            staging.map(|row| row.id.clone()).expect("staging is saved")
        });
        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                let shell = gpui_kit::base::Root::read(window, cx)
                    .view()
                    .clone()
                    .downcast::<ui::shell::Shell>()
                    .expect("the window shows the shell");
                let navigator = shell
                    .read(cx)
                    .workspace()
                    .expect("the workspace is open")
                    .read(cx)
                    .navigator()
                    .clone();
                navigator.update(cx, |navigator, cx| {
                    navigator.connect(shop.clone(), cx);
                    navigator.connect(failing, cx);
                });
            })
            .ok();
        stage.settle(Duration::from_millis(500));
        stage.shoot("navigator");
        // The failure toast has been seen; keep it off the pages that follow.
        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                use gpui_kit::component::WindowExt as _;
                window.clear_notifications(cx)
            })
            .ok();

        // The connection is production, so writing SQL asks first.
        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                let shell = gpui_kit::base::Root::read(window, cx)
                    .view()
                    .clone()
                    .downcast::<ui::shell::Shell>()
                    .expect("the window shows the shell");
                let workspace = shell.read(cx).workspace().expect("the workspace is open");
                let editor = workspace
                    .read(cx)
                    .active_editor()
                    .expect("an editor is open");
                editor.update(cx, |editor, cx| {
                    editor.set_text("DELETE FROM orders WHERE total < 50;", window, cx);
                    editor.run(window, cx);
                });
            })
            .ok();
        stage.settle(Duration::from_millis(300));
        stage.shoot("editor-production");
        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                use gpui_kit::component::WindowExt as _;
                window.close_all_dialogs(cx)
            })
            .ok();

        // The open connection got an editor; run SQL in it, then SQL that fails.
        for (name, sql) in [
            (
                "editor-results",
                "SELECT c.name, c.email, o.total, o.note\nFROM orders o\nJOIN customers c ON c.id = o.customer_id\nORDER BY o.total DESC;",
            ),
            (
                "editor-error",
                "UPDATE orders SET note = 'checked' WHERE total > 100;\nSELECT * FROM orders WHERE;",
            ),
            (
                "editor-transaction",
                "BEGIN;\nUPDATE orders SET note = 'held' WHERE id = 1;",
            ),
            // Leaves no transaction open for the pages that follow.
            ("", "ROLLBACK;"),
        ] {
            stage
                .cx
                .update_window(stage.window, |_, window, cx| {
                    let shell = gpui_kit::base::Root::read(window, cx)
                        .view()
                        .clone()
                        .downcast::<ui::shell::Shell>()
                        .expect("the window shows the shell");
                    let workspace = shell.read(cx).workspace().expect("the workspace is open");
                    let editor = workspace.read(cx).active_editor().expect("an editor is open");
                    editor.update(cx, |editor, cx| {
                        editor.set_text(sql, window, cx);
                        editor.start(sql.to_string(), 0, window, cx);
                    });
                })
                .ok();
            stage.settle(Duration::from_millis(500));
            if !name.is_empty() {
                stage.shoot(name);
            }
        }

        // A table tab: its rows, then staged edits, then its structure.
        let workspace = |window: &gpui_kit::Window, cx: &gpui_kit::App| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<ui::shell::Shell>()
                .expect("the window shows the shell")
                .read(cx)
                .workspace()
                .expect("the workspace is open")
                .clone()
        };
        let table = desktop_lib::db::TableRef {
            schema: None,
            name: "customers".to_string(),
        };
        let connection = shop.clone();
        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                workspace(window, cx).update(cx, |workspace, cx| {
                    workspace.open_table(connection, table, window, cx)
                });
            })
            .ok();
        stage.settle(Duration::from_millis(500));
        stage.shoot("table-data");

        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                let view = workspace(window, cx)
                    .read(cx)
                    .active_table()
                    .expect("a table tab");
                view.update(cx, |view, cx| {
                    view.stage(0, 1, Some("Ann Lee-Park".to_string()), cx);
                    view.stage(1, 2, Some("bo@example.com".to_string()), cx);
                    view.toggle_delete(2, cx);
                    view.add_row(cx);
                    view.stage(3, 1, Some("Dee Wu".to_string()), cx);
                });
            })
            .ok();
        stage.settle(Duration::from_millis(300));
        stage.shoot("table-pending");

        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                let view = workspace(window, cx)
                    .read(cx)
                    .active_table()
                    .expect("a table tab");
                view.update(cx, |view, cx| view.show_structure(window, cx));
            })
            .ok();
        stage.settle(Duration::from_millis(500));
        stage.shoot("table-structure");

        let warehouse = stage.store(|store, _| {
            let saved = store.connections.iter().find(|row| row.name == "warehouse");
            saved.map(|row| row.id.clone())
        });
        for (name, route) in [
            ("connection-new", Route::ConnectionForm(None)),
            ("connection-postgres", Route::ConnectionForm(warehouse)),
            ("connection-edit", Route::ConnectionForm(Some(shop))),
            ("ssh-hosts", Route::SshHosts),
            ("ssh-host-new", Route::SshHostForm(None)),
            ("ssh-keys", Route::SshKeys),
            ("ssh-key-new", Route::SshKeyForm(None)),
            ("kubeconfigs", Route::Kubeconfigs),
            ("kubeconfig-new", Route::KubeconfigForm(None)),
            (
                "ssh-key-missing",
                Route::SshKeyForm(Some("gone".to_string())),
            ),
        ] {
            stage.store(|store, cx| store.navigate(route, cx));
            stage.settle(Duration::from_millis(300));
            stage.shoot(name);
        }

        stage.store(|store, cx| store.navigate(Route::Settings, cx));
        stage.settle(Duration::from_millis(500));
        stage.shoot("settings");

        // A port of its own, so a running noverdbee does not hold it.
        desktop_lib::app::runtime().block_on(async {
            let port = shared::Setting {
                key: "noverdbee.mcp.port".to_string(),
                value: (desktop_lib::mcp::DEFAULT_PORT + 9).to_string(),
            };
            desktop_lib::store::settings::set_setting(port, app.state())
                .await
                .expect("save MCP port");
            desktop_lib::mcp::start(app.clone()).await
        });
        stage.store(|store, cx| store.navigate(Route::Mcp, cx));
        stage.settle(Duration::from_millis(500));
        stage.shoot("mcp");

        let release = ui::views::updater::Release {
            version: "9.9.9".to_string(),
            notes: Some("- Faster table rendering\n- Fixes for wide results".to_string()),
            platforms: Default::default(),
        };
        stage
            .cx
            .update_window(stage.window, |_, window, cx| {
                ui::views::updater::show(release, window, cx)
            })
            .ok();
        stage.settle(Duration::from_millis(500));
        stage.shoot("update");
        stage.close_dialogs();
    }
}
