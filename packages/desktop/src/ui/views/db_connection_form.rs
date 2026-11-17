//! Add or edit a saved database connection, and test it before saving.

use std::time::Instant;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, Disableable as _};
use gpui_kit::*;
use shared::DbConnectionRecord;

use crate::db::{Session, SessionManager, DRIVER_SQLITE};
use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::common;

#[derive(Debug, Default, PartialEq)]
struct Errors {
    name: Option<&'static str>,
    path: Option<&'static str>,
}

fn validate(name: &str, path: &str) -> Errors {
    Errors {
        name: name.trim().is_empty().then_some("Name is required"),
        path: path.trim().is_empty().then_some("Choose a database file"),
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
    name: Entity<InputState>,
    group: Entity<InputState>,
    path: Entity<InputState>,
    read_only: bool,
    errors: Errors,
    error: Option<SharedString>,
    saving: bool,
    test: Test,
}

impl DbConnectionForm {
    pub fn new(id: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let (existing, group_name) = {
            let store = store.read(cx);
            let existing = id.as_deref().and_then(|id| store.connection(id)).cloned();
            let group_name = existing
                .as_ref()
                .and_then(|record| record.group_id.as_deref())
                .and_then(|group_id| store.groups.iter().find(|group| group.id == group_id))
                .map(|group| group.name.clone())
                .unwrap_or_default();
            (existing.unwrap_or_default(), group_name)
        };
        let mut input = |value: String, placeholder: SharedString| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder(placeholder)
            })
        };
        Self {
            name: input(existing.name, t("orders")),
            group: input(group_name, t("production")),
            path: input(existing.path, t("/path/to/database.db")),
            read_only: existing.read_only,
            store,
            id,
            errors: Errors::default(),
            error: None,
            saving: false,
            test: Test::Idle,
        }
    }

    fn record(&self, cx: &App) -> DbConnectionRecord {
        DbConnectionRecord {
            id: self.id.clone().unwrap_or_default(),
            name: self.name.read(cx).value().trim().to_string(),
            driver: DRIVER_SQLITE.to_string(),
            path: self.path.read(cx).value().trim().to_string(),
            read_only: self.read_only,
            ..DbConnectionRecord::default()
        }
    }

    /// Checks the fields; `false` when the form cannot be used yet.
    fn check(&mut self, cx: &mut Context<Self>) -> bool {
        let record = self.record(cx);
        self.errors = validate(&record.name, &record.path);
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
                t("Name"),
                Input::new(&self.name),
                self.errors.name.map(t),
            ))
            .child(common::field(t("Group"), Input::new(&self.group), None));
        let file = common::form_section("file", t("SQLite file"), None)
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
            );
        let testing = matches!(self.test, Test::Running);

        common::form_page(
            title,
            self.error.clone(),
            v_flex()
                .gap_4()
                .child(connection)
                .child(file)
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

    #[test]
    fn a_connection_needs_a_name_and_a_file() {
        assert_eq!(validate("orders", "/tmp/a.db"), Errors::default());
        assert_eq!(
            validate("  ", ""),
            Errors {
                name: Some("Name is required"),
                path: Some("Choose a database file"),
            }
        );
    }
}
