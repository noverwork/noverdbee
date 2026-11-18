//! SSH keys: the list, and the form to add or edit one. Laid out like
//! noverterm's Keys page.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Sizable as _};
use gpui_kit::*;
use shared::SshKeyRecord;

use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::common;

const KEY_PLACEHOLDER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";

/// The fingerprint, cut to fit a card.
fn short_fingerprint(fingerprint: &str) -> SharedString {
    if fingerprint.is_empty() {
        "—".into()
    } else if fingerprint.chars().count() > 24 {
        format!("{}…", fingerprint.chars().take(24).collect::<String>()).into()
    } else {
        fingerprint.to_string().into()
    }
}

// --- List --------------------------------------------------------------------

pub struct SshKeysView {
    store: Entity<Store>,
    search: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl SshKeysView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let (search, searched) = common::new_search(t("Search keys"), window, cx);
        let _subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify()), searched];
        Self {
            store,
            search,
            error: None,
            _subscriptions,
        }
    }

    fn open(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.store
            .update(cx, |store, cx| store.navigate(Route::SshKeyForm(id), cx));
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
            t("Delete SSH key?"),
            t("SSH hosts using this key will sign in with their password only."),
            Some(name.into()),
            t("Delete key"),
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
                    async move { crate::store::tunnels::ssh_key_delete(id, app.state()).await },
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

impl Render for SshKeysView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let query = self.search.read(cx).value();
        let cards = store
            .ssh_keys
            .iter()
            .filter(|item| common::matches(&query, &[&item.name, &item.fingerprint]))
            .map(|item| {
                let (open, delete, name) = (item.id.clone(), item.id.clone(), item.name.clone());
                common::item_card(
                    &item.id,
                    "key-round",
                    &item.name,
                    vec![short_fingerprint(&item.fingerprint)],
                    cx.listener(move |this, _, _, cx| this.open(Some(open.clone()), cx)),
                    cx.listener(move |this, _, window, cx| {
                        this.request_delete(delete.clone(), name.clone(), window, cx)
                    }),
                )
                .into_any_element()
            })
            .collect();
        common::list_page(
            t("SSH Keys"),
            &self.search,
            t("Add key"),
            cx.listener(|this, _, _, cx| this.open(None, cx)),
            self.error.clone(),
            if store.ssh_keys.is_empty() {
                t("No SSH keys saved yet")
            } else {
                t("No keys match your search.")
            },
            cards,
        )
    }
}

// --- Form --------------------------------------------------------------------

/// What is wrong with the fields, if anything.
fn validate(record: &SshKeyRecord) -> Option<&'static str> {
    if record.name.is_empty() {
        Some("Key name is required")
    } else if record.private_key.is_empty() {
        Some("Private key is required")
    } else {
        None
    }
}

pub struct SshKeyForm {
    store: Entity<Store>,
    /// The one being edited; `None` adds a new one.
    id: Option<String>,
    /// Editing one that no longer exists.
    missing: bool,
    /// The saved fingerprint, when editing.
    fingerprint: String,
    passphrase: Entity<InputState>,
    name: Entity<InputState>,
    private_key: Entity<TextareaState>,
    error: Option<SharedString>,
    saving: bool,
}

impl SshKeyForm {
    pub fn new(id: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let existing = id
            .as_deref()
            .and_then(|id| {
                let store = store.read(cx);
                store.ssh_keys.iter().find(|item| item.id == id)
            })
            .cloned()
            .unwrap_or_default();
        Self {
            missing: id.is_some() && existing.id.is_empty(),
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(existing.name)
                    .placeholder(tf("e.g. {example}", &[("example", "GitHub key")]))
            }),
            private_key: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(10)
                    .default_value(existing.private_key)
                    .placeholder(KEY_PLACEHOLDER)
            }),
            passphrase: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .default_value(existing.passphrase)
            }),
            fingerprint: existing.fingerprint,
            store,
            id,
            error: None,
            saving: false,
        }
    }

    fn record(&self, cx: &App) -> SshKeyRecord {
        SshKeyRecord {
            id: self.id.clone().unwrap_or_default(),
            name: self.name.read(cx).value().trim().to_string(),
            private_key: self.private_key.read(cx).value().trim().to_string(),
            // A passphrase may begin or end with a space.
            passphrase: self.passphrase.read(cx).value().to_string(),
            // Worked out when the key is saved.
            fingerprint: String::new(),
        }
    }

    /// Checks the fields; `false` when the form cannot be saved yet.
    fn check(&mut self, cx: &mut Context<Self>) -> bool {
        self.error = validate(&self.record(cx)).map(t);
        cx.notify();
        self.error.is_none()
    }

    /// Fills the private key from a file the user picks.
    fn import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("Import")),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let text = backend(async move {
                tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match text {
                    Ok(text) => {
                        this.error = None;
                        this.private_key
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.store
            .update(cx, |store, cx| store.navigate(Route::SshKeys, cx));
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
            let saved =
                backend(
                    async move { crate::store::tunnels::ssh_key_save(record, app.state()).await },
                )
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

impl Render for SshKeyForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.missing {
            return common::not_found(
                t("SSH key not found"),
                t("Back to SSH keys"),
                cx.listener(|this, _, _, cx| this.back(cx)),
            );
        }
        let editing = self.id.is_some();
        let saving = self.saving;
        let submit_label = match (saving, editing) {
            (true, true) => t("Updating…"),
            (true, false) => t("Saving…"),
            (false, true) => t("Update key"),
            (false, false) => t("Save key"),
        };
        let fingerprint = editing.then(|| {
            v_flex()
                .gap_1()
                .px_3()
                .py_2p5()
                .rounded(theme::RADIUS)
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::RAIL)
                .child(common::section_title(t("Current fingerprint")))
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .text_color(theme::STENCIL_DIM)
                        .child(self.fingerprint.clone()),
                )
        });
        common::form_page(
            if editing {
                t("Edit SSH key")
            } else {
                t("New SSH key")
            },
            self.error.clone(),
            v_flex()
                .gap_4()
                .child(
                    common::form_section("key-round", t("Key identity"), None)
                        .children(fingerprint)
                        .child(common::field(
                            t("Key name"),
                            Input::new(&self.name).disabled(saving),
                            None,
                        ))
                        .child(common::field(
                            SharedString::from(format!("{} {}", t("Passphrase"), t("(optional)"))),
                            Input::new(&self.passphrase).disabled(saving),
                            None,
                        )),
                )
                .child(
                    common::form_section("key-round", t("Private key"), None)
                        .child(
                            h_flex().child(
                                Button::new("import-key")
                                    .outline()
                                    .small()
                                    .icon(lucide("file-input"))
                                    .label(t("Import from file"))
                                    .disabled(saving)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_file(window, cx)
                                    })),
                            ),
                        )
                        .child(Textarea::new(&self.private_key).disabled(saving)),
                ),
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
    use shared::SshKeyRecord;

    #[test]
    fn the_name_is_asked_for_before_the_private_key() {
        let blank = SshKeyRecord::default();
        assert_eq!(validate(&blank), Some("Key name is required"));
        let named = SshKeyRecord {
            name: "a".to_string(),
            ..blank
        };
        assert_eq!(validate(&named), Some("Private key is required"));
        let filled = SshKeyRecord {
            private_key: "b".to_string(),
            ..named
        };
        assert_eq!(validate(&filled), None);
    }

    #[test]
    fn a_long_fingerprint_is_cut_to_fit_a_card() {
        use super::short_fingerprint;
        assert_eq!(short_fingerprint(""), "—");
        assert_eq!(short_fingerprint("SHA256:abc"), "SHA256:abc");
        assert_eq!(
            short_fingerprint("SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8"),
            "SHA256:nThbg6kXUpJWGl7E1…"
        );
    }
}
