//! Kubeconfigs: the list, and the form to add or edit one. Laid out like
//! noverterm's Keys page.

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::*;
use shared::KubeconfigRecord;

use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::store::{store, Route, Store};
use crate::ui::views::common;

/// The `current-context` the YAML names, `—` when it names none.
fn current_context(config: &str) -> SharedString {
    config
        .lines()
        .find_map(|line| line.strip_prefix("current-context:"))
        .map(|context| context.trim().trim_matches(['"', '\'']).to_string())
        .filter(|context| !context.is_empty())
        .unwrap_or_else(|| "—".to_string())
        .into()
}

// --- List --------------------------------------------------------------------

pub struct KubeconfigsView {
    store: Entity<Store>,
    search: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl KubeconfigsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let (search, searched) = common::new_search(t("Search kubeconfigs"), window, cx);
        let _subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify()), searched];
        Self {
            store,
            search,
            error: None,
            _subscriptions,
        }
    }

    fn open(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| {
            store.navigate(Route::KubeconfigForm(id), cx)
        });
    }

    fn request_delete(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = None;
        cx.notify();
        let view = cx.entity();
        common::confirm_delete(
            t("Delete kubeconfig?"),
            t("Connections using this kubeconfig cannot connect until they pick another."),
            Some(name.into()),
            t("Delete kubeconfig"),
            window,
            cx,
            move |_, cx| view.update(cx, |this, cx| this.delete(id.clone(), cx)),
        );
    }

    fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        let app = self.store.read(cx).app();
        cx.spawn(async move |this, cx| {
            let deleted =
                backend(
                    async move { crate::store::tunnels::kubeconfig_delete(id, app.state()).await },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.error = deleted.err().map(SharedString::from);
                this.store.update(cx, |store, cx| store.refresh(cx));
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for KubeconfigsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let query = self.search.read(cx).value();
        let cards = store
            .kubeconfigs
            .iter()
            .filter(|item| {
                common::matches(
                    &query,
                    &[&item.name, current_context(&item.config).as_ref()],
                )
            })
            .map(|item| {
                let (open, delete, name) = (item.id.clone(), item.id.clone(), item.name.clone());
                common::item_card(
                    &item.id,
                    "ship-wheel",
                    &item.name,
                    vec![current_context(&item.config)],
                    cx.listener(move |this, _, _, cx| this.open(Some(open.clone()), cx)),
                    cx.listener(move |this, _, window, cx| {
                        this.request_delete(delete.clone(), name.clone(), window, cx)
                    }),
                )
                .into_any_element()
            })
            .collect();
        common::list_page(
            t("Kubeconfigs"),
            &self.search,
            t("Add kubeconfig"),
            cx.listener(|this, _, _, cx| this.open(None, cx)),
            self.error.clone(),
            if store.kubeconfigs.is_empty() {
                t("No kubeconfigs saved yet")
            } else {
                t("No kubeconfigs match your search.")
            },
            cards,
        )
    }
}

// --- Form --------------------------------------------------------------------

/// What is wrong with the fields, if anything.
fn validate(record: &KubeconfigRecord) -> Option<&'static str> {
    if record.name.is_empty() {
        Some("Kubeconfig name is required")
    } else if record.config.is_empty() {
        Some("Kubeconfig is required")
    } else {
        None
    }
}

pub struct KubeconfigForm {
    store: Entity<Store>,
    /// The one being edited; `None` adds a new one.
    id: Option<String>,
    /// Editing one that no longer exists.
    missing: bool,
    name: Entity<InputState>,
    config: Entity<TextareaState>,
    error: Option<SharedString>,
    saving: bool,
}

impl KubeconfigForm {
    pub fn new(id: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let existing = id
            .as_deref()
            .and_then(|id| {
                let store = store.read(cx);
                store.kubeconfigs.iter().find(|item| item.id == id)
            })
            .cloned()
            .unwrap_or_default();
        Self {
            missing: id.is_some() && existing.id.is_empty(),
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(existing.name)
                    .placeholder(tf("e.g. {example}", &[("example", "staging")]))
            }),
            config: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(16)
                    .default_value(existing.config)
                    .placeholder("apiVersion: v1\nkind: Config\n...")
            }),
            store,
            id,
            error: None,
            saving: false,
        }
    }

    fn record(&self, cx: &App) -> KubeconfigRecord {
        KubeconfigRecord {
            id: self.id.clone().unwrap_or_default(),
            name: self.name.read(cx).value().trim().to_string(),
            config: self.config.read(cx).value().trim().to_string(),
        }
    }

    /// Checks the fields; `false` when the form cannot be saved yet.
    fn check(&mut self, cx: &mut Context<Self>) -> bool {
        self.error = validate(&self.record(cx)).map(t);
        cx.notify();
        self.error.is_none()
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.close_form(cx));
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || !self.check(cx) {
            return;
        }
        self.saving = true;
        self.error = None;
        cx.notify();
        let record = self.record(cx);
        let app = self.store.read(cx).app();
        cx.spawn(async move |this, cx| {
            let saved = backend(async move {
                crate::store::tunnels::kubeconfig_save(record, app.state()).await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match saved {
                    Ok(_) => {
                        this.store.update(cx, |store, cx| store.refresh(cx));
                        this.back(cx);
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for KubeconfigForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.missing {
            return common::not_found(
                t("Kubeconfig not found"),
                t("Back to kubeconfigs"),
                cx.listener(|this, _, _, cx| this.back(cx)),
            );
        }
        let editing = self.id.is_some();
        let saving = self.saving;
        let submit_label = match (saving, editing) {
            (true, true) => t("Updating…"),
            (true, false) => t("Saving…"),
            (false, true) => t("Update kubeconfig"),
            (false, false) => t("Save kubeconfig"),
        };

        common::form_page(
            self.error.clone(),
            common::form_section(None, None)
                .child(common::field(
                    t("Name"),
                    Input::new(&self.name).disabled(saving),
                    None,
                ))
                .child(common::field(
                    t("Kubeconfig"),
                    Textarea::new(&self.config).disabled(saving),
                    None,
                )),
            common::form_buttons(
                submit_label,
                saving,
                cx.listener(|this, _, _, cx| this.back(cx)),
                cx.listener(|this, _, _, cx| this.save(cx)),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would pull in GPUI's own `test` attribute.
    use super::validate;
    use shared::KubeconfigRecord;

    #[test]
    fn the_name_is_asked_for_before_the_config() {
        let blank = KubeconfigRecord::default();
        assert_eq!(validate(&blank), Some("Kubeconfig name is required"));
        let named = KubeconfigRecord {
            name: "a".to_string(),
            ..blank
        };
        assert_eq!(validate(&named), Some("Kubeconfig is required"));
        let filled = KubeconfigRecord {
            config: "b".to_string(),
            ..named
        };
        assert_eq!(validate(&filled), None);
    }

    #[test]
    fn a_kubeconfig_is_described_by_its_current_context() {
        use super::current_context;
        let config = "apiVersion: v1\ncurrent-context: \"staging\"\nkind: Config\n";
        assert_eq!(current_context(config), "staging");
        assert_eq!(current_context("apiVersion: v1\n"), "—");
    }
}
