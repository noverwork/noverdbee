//! Add or edit a saved database connection, and test it before saving.

use std::time::Instant;

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use shared::{DbConnectionRecord, Environment, McpAccess};

use crate::db::{Session, SessionManager, DRIVER_POSTGRES, DRIVER_SQLITE};
use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::common::{self, Choice, ChoiceSelect};

const DEFAULT_PORT: i32 = 5432;

#[derive(Debug, Default, PartialEq)]
struct Errors {
    name: Option<&'static str>,
    path: Option<&'static str>,
    host: Option<&'static str>,
    port: Option<&'static str>,
    username: Option<&'static str>,
}

/// Checks what the record's driver needs. `port` is the text as typed.
fn validate(record: &DbConnectionRecord, port: &str) -> Errors {
    let missing = |value: &str, message: &'static str| value.trim().is_empty().then_some(message);
    let postgres = record.driver == DRIVER_POSTGRES;
    Errors {
        name: missing(&record.name, "Name is required"),
        path: (!postgres)
            .then(|| missing(&record.path, "Choose a database file"))
            .flatten(),
        host: postgres
            .then(|| missing(&record.host, "Host is required"))
            .flatten(),
        port: (postgres && !matches!(port.trim().parse::<u16>(), Ok(1..)))
            .then_some("Port must be a number from 1 to 65535"),
        username: postgres
            .then(|| missing(&record.username, "User is required"))
            .flatten(),
    }
}

enum Test {
    Idle,
    Running,
    Passed(SharedString),
    Failed(SharedString),
}

pub struct DbConnectionForm {
    store: Entity<Store>,
    /// The connection being edited; `None` adds a new one.
    id: Option<String>,
    driver: Entity<ChoiceSelect>,
    name: Entity<InputState>,
    group: Entity<InputState>,
    environment: Entity<ChoiceSelect>,
    path: Entity<InputState>,
    read_only: bool,
    mcp: McpAccess,
    host: Entity<InputState>,
    port: Entity<InputState>,
    database: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    ssl_mode: Entity<ChoiceSelect>,
    errors: Errors,
    error: Option<SharedString>,
    saving: bool,
    test: Test,
    _subscriptions: Vec<Subscription>,
}

impl DbConnectionForm {
    /// Edits connection `id`, or adds one, in `group` when given.
    pub fn new(
        id: Option<String>,
        group: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = store(cx);
        let (existing, group_name) = {
            let store = store.read(cx);
            let existing = id.as_deref().and_then(|id| store.connection(id)).cloned();
            let group_name = existing
                .as_ref()
                .and_then(|record| record.group_id.as_deref())
                .and_then(|group_id| store.groups.iter().find(|group| group.id == group_id))
                .map(|group| group.name.clone())
                .or(group)
                .unwrap_or_default();
            let new = DbConnectionRecord {
                driver: DRIVER_SQLITE.to_string(),
                port: DEFAULT_PORT,
                ssl_mode: "prefer".to_string(),
                ..DbConnectionRecord::default()
            };
            (existing.unwrap_or(new), group_name)
        };
        let driver = common::new_select(
            vec![
                Choice::new(DRIVER_SQLITE, "SQLite"),
                Choice::new(DRIVER_POSTGRES, "PostgreSQL"),
            ],
            &existing.driver,
            window,
            cx,
        );
        let ssl_mode = common::new_select(
            ["disable", "prefer", "require"]
                .map(|mode| Choice::new(mode, mode))
                .to_vec(),
            if existing.ssl_mode.is_empty() {
                "prefer"
            } else {
                &existing.ssl_mode
            },
            window,
            cx,
        );
        let environment = common::new_select(
            Environment::ALL
                .map(|environment| {
                    Choice::new(environment.as_str(), environment_label(environment))
                })
                .to_vec(),
            existing.environment.as_str(),
            window,
            cx,
        );
        // Switching driver swaps the sections and clears their messages.
        let _subscriptions =
            vec![
                cx.subscribe(&driver, |this, _, _: &SelectEvent<Vec<Choice>>, cx| {
                    this.errors = Errors::default();
                    this.test = Test::Idle;
                    cx.notify();
                }),
            ];
        let mut input = |value: String, placeholder: &'static str| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder(placeholder)
            })
        };
        let port = if existing.port > 0 {
            existing.port
        } else {
            DEFAULT_PORT
        };
        Self {
            name: input(existing.name, "orders"),
            group: input(group_name, "analytics"),
            environment,
            path: input(existing.path, "/path/to/database.db"),
            host: input(existing.host, "localhost"),
            port: input(port.to_string(), "5432"),
            database: input(existing.database, "postgres"),
            username: input(existing.username, "postgres"),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .default_value(existing.password)
            }),
            read_only: existing.read_only,
            mcp: existing.mcp,
            driver,
            ssl_mode,
            store,
            id,
            errors: Errors::default(),
            error: None,
            saving: false,
            test: Test::Idle,
            _subscriptions,
        }
    }

    fn is_postgres(&self, cx: &App) -> bool {
        common::selected(&self.driver, cx).as_deref() == Some(DRIVER_POSTGRES)
    }

    /// The record the fields describe; fields of the other driver stay empty.
    fn record(&self, cx: &App) -> DbConnectionRecord {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let shared = DbConnectionRecord {
            id: self.id.clone().unwrap_or_default(),
            name: text(&self.name),
            environment: Environment::parse(
                &common::selected(&self.environment, cx).unwrap_or_default(),
            ),
            mcp: self.mcp,
            ..DbConnectionRecord::default()
        };
        if self.is_postgres(cx) {
            DbConnectionRecord {
                driver: DRIVER_POSTGRES.to_string(),
                host: text(&self.host),
                port: text(&self.port).parse().unwrap_or(0),
                database: text(&self.database),
                username: text(&self.username),
                // A password may begin or end with a space.
                password: self.password.read(cx).value().to_string(),
                ssl_mode: common::selected(&self.ssl_mode, cx).unwrap_or_default(),
                ..shared
            }
        } else {
            DbConnectionRecord {
                driver: DRIVER_SQLITE.to_string(),
                path: text(&self.path),
                read_only: self.read_only,
                ..shared
            }
        }
    }

    /// Checks the fields; `false` when the form cannot be used yet.
    fn check(&mut self, cx: &mut Context<Self>) -> bool {
        self.errors = validate(&self.record(cx), &self.port.read(cx).value());
        cx.notify();
        self.errors == Errors::default()
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.store
            .update(cx, |store, cx| store.navigate(Route::Databases, cx));
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("Choose")),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let path = path.to_string_lossy().into_owned();
                this.path
                    .update(cx, |input, cx| input.set_value(path, window, cx));
                this.test = Test::Idle;
                cx.notify();
            });
        })
        .detach();
    }

    fn run_test(&mut self, cx: &mut Context<Self>) {
        if matches!(self.test, Test::Running) || !self.check(cx) {
            return;
        }
        let record = self.record(cx);
        self.test = Test::Running;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let tested =
                backend(async move { Session::open(&record).await?.version().await }).await;
            let elapsed = started.elapsed().as_millis().to_string();
            let _ = this.update(cx, |this, cx| {
                this.test = match tested {
                    Ok(version) => Test::Passed(tf(
                        "Connected · {version} · {elapsed} ms",
                        &[("version", &version), ("elapsed", &elapsed)],
                    )),
                    Err(error) => Test::Failed(error.into()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || !self.check(cx) {
            return;
        }
        self.saving = true;
        self.error = None;
        cx.notify();
        let record = self.record(cx);
        let group = self.group.read(cx).value().to_string();
        let app = self.store.read(cx).app();
        cx.spawn(async move |this, cx| {
            let saved = backend(async move {
                let saved =
                    crate::store::connections::connection_save(record, group, app.state()).await?;
                // An open session still uses the old settings.
                app.state::<SessionManager>().disconnect(&saved.id);
                Ok::<_, String>(())
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match saved {
                    Ok(()) => {
                        this.store.update(cx, |store, cx| store.refresh(cx));
                        this.close(cx);
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_test(&self) -> Option<Div> {
        let (text, failed) = match &self.test {
            Test::Idle | Test::Running => return None,
            Test::Passed(text) => (text.clone(), false),
            Test::Failed(text) => (text.clone(), true),
        };
        let panel = div()
            .px_4()
            .py_3()
            .rounded(theme::RADIUS)
            .border_1()
            .text_sm();
        Some(if failed {
            panel
                .border_color(theme::SEAL.opacity(0.6))
                .text_color(theme::RED_TEXT)
                .child(text)
        } else {
            panel
                .border_color(theme::hairline())
                .text_color(theme::STENCIL_DIM)
                .child(text)
        })
    }

    fn render_file(&self, cx: &Context<Self>) -> Div {
        common::form_section("file", t("SQLite file"), None)
            .child(common::field(
                t("Path"),
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.path)))
                    .child(
                        Button::new("browse")
                            .outline()
                            .label(t("Browse"))
                            .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))),
                    ),
                self.errors.path.map(t),
            ))
            .child(
                Switch::new("read-only")
                    .checked(self.read_only)
                    .label(t("Open read-only"))
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        this.read_only = *checked;
                        cx.notify();
                    })),
            )
    }

    /// The group field, with the existing groups to pick from so a typo does
    /// not start a second group.
    fn render_group(&self, cx: &Context<Self>) -> Div {
        let groups = &self.store.read(cx).groups;
        v_flex()
            .gap_2()
            .child(Input::new(&self.group))
            .when(!groups.is_empty(), |field| {
                field.child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .children(groups.iter().map(|group| {
                            let name = group.name.clone();
                            Button::new(SharedString::from(format!("group-{}", group.id)))
                                .outline()
                                .xsmall()
                                .label(group.name.clone())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let name = name.clone();
                                    this.group
                                        .update(cx, |input, cx| input.set_value(name, window, cx));
                                }))
                        })),
                )
            })
    }

    fn render_server(&self) -> Div {
        common::form_section("server", t("Server"), None)
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(common::field(
                        t("Host"),
                        Input::new(&self.host),
                        self.errors.host.map(t),
                    )))
                    .child(div().w_32().child(common::field(
                        t("Port"),
                        Input::new(&self.port),
                        self.errors.port.map(t),
                    ))),
            )
            .child(common::field(
                t("Database"),
                Input::new(&self.database),
                None,
            ))
            .child(common::field(
                t("User"),
                Input::new(&self.username),
                self.errors.username.map(t),
            ))
            .child(common::field(
                t("Password"),
                Input::new(&self.password).mask_toggle(),
                None,
            ))
            .child(common::field(
                t("SSL mode"),
                Select::new(&self.ssl_mode),
                None,
            ))
    }
}

impl DbConnectionForm {
    /// Which MCP tools AI clients may use here; all off keeps it out of MCP.
    fn render_mcp(&self, cx: &Context<Self>) -> Div {
        type Flag = fn(&mut McpAccess) -> &mut bool;
        let tools: [(&'static str, Flag); 4] = [
            ("select", |access| &mut access.select),
            ("insert", |access| &mut access.insert),
            ("update", |access| &mut access.update),
            ("delete", |access| &mut access.delete),
        ];
        let mut mcp = self.mcp;
        common::form_section(
            "plug",
            t("AI access (MCP)"),
            Some(t(
                "Tools AI clients may use on this connection. All off keeps it hidden.",
            )),
        )
        .child(h_flex().gap_6().children(tools.map(|(name, flag)| {
            Checkbox::new(SharedString::from(format!("mcp-{name}")))
                .label(name)
                .checked(*flag(&mut mcp))
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    *flag(&mut this.mcp) = *checked;
                    cx.notify();
                }))
        })))
    }
}

fn environment_label(environment: Environment) -> SharedString {
    match environment {
        Environment::Local => t("Local"),
        Environment::Development => t("Development"),
        Environment::Staging => t("Staging"),
        Environment::Production => t("Production"),
    }
}

impl Render for DbConnectionForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = if self.id.is_some() {
            t("Edit connection")
        } else {
            t("New connection")
        };
        let connection = common::form_section("database", t("Connection"), None)
            .child(common::field(
                t("Database type"),
                Select::new(&self.driver),
                None,
            ))
            .child(common::field(
                t("Name"),
                Input::new(&self.name),
                self.errors.name.map(t),
            ))
            .child(common::field(
                t("Environment"),
                Select::new(&self.environment),
                None,
            ))
            .child(common::field(t("Group"), self.render_group(cx), None));
        let details = if self.is_postgres(cx) {
            self.render_server()
        } else {
            self.render_file(cx)
        };
        let testing = matches!(self.test, Test::Running);

        common::form_page(
            title,
            self.error.clone(),
            v_flex()
                .gap_4()
                .child(connection)
                .child(details)
                .child(self.render_mcp(cx))
                .children(self.render_test()),
            h_flex()
                .w_full()
                .justify_between()
                .child(
                    Button::new("test-connection")
                        .outline()
                        .label(t("Test connection"))
                        .loading(testing)
                        .disabled(testing)
                        .on_click(cx.listener(|this, _, _, cx| this.run_test(cx))),
                )
                .child(common::form_buttons(
                    t("Save"),
                    self.saving,
                    cx.listener(|this, _, _, cx| this.close(cx)),
                    cx.listener(|this, _, _, cx| this.save(cx)),
                )),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would pull in GPUI's own `test` attribute.
    use super::{validate, Errors};
    use crate::db::{DRIVER_POSTGRES, DRIVER_SQLITE};
    use shared::DbConnectionRecord;

    #[test]
    fn each_driver_asks_only_for_its_own_fields() {
        let sqlite = DbConnectionRecord {
            name: "orders".to_string(),
            driver: DRIVER_SQLITE.to_string(),
            path: "/tmp/a.db".to_string(),
            ..Default::default()
        };
        assert_eq!(validate(&sqlite, ""), Errors::default());
        let blank = DbConnectionRecord {
            driver: DRIVER_SQLITE.to_string(),
            ..Default::default()
        };
        assert_eq!(
            validate(&blank, "5432"),
            Errors {
                name: Some("Name is required"),
                path: Some("Choose a database file"),
                ..Errors::default()
            }
        );

        let postgres = DbConnectionRecord {
            name: "orders".to_string(),
            driver: DRIVER_POSTGRES.to_string(),
            host: "db.internal".to_string(),
            username: "app".to_string(),
            ..Default::default()
        };
        assert_eq!(validate(&postgres, "5432"), Errors::default());
        let incomplete = DbConnectionRecord {
            host: " ".to_string(),
            username: String::new(),
            ..postgres
        };
        for port in ["", "0", "70000", "abc"] {
            assert_eq!(
                validate(&incomplete, port),
                Errors {
                    host: Some("Host is required"),
                    port: Some("Port must be a number from 1 to 65535"),
                    username: Some("User is required"),
                    ..Errors::default()
                },
                "port {port:?}"
            );
        }
    }
}
