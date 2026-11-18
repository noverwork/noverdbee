//! SSH hosts connections tunnel through: the list, and the form to add or
//! edit one. Laid out like noverterm's Connections page.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, Disableable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use shared::SshHostRecord;

use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::common::{self, Choice, ChoiceSelect};

const DEFAULT_PORT: i32 = 22;

/// How the host signs in, for its card.
fn auth_label(host: &SshHostRecord) -> SharedString {
    match (host.key_id.is_some(), host.password.is_empty()) {
        (true, false) => t("Key + Password"),
        (true, true) => t("SSH Key"),
        (false, _) => t("Password"),
    }
}

// --- List --------------------------------------------------------------------

pub struct SshHostsView {
    store: Entity<Store>,
    search: Entity<InputState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl SshHostsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let (search, searched) = common::new_search(t("Search hosts"), window, cx);
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
            .update(cx, |store, cx| store.navigate(Route::SshHostForm(id), cx));
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
            t("Delete SSH host?"),
            t("Connections tunnelling through this host cannot connect until they pick another."),
            Some(name.into()),
            t("Delete host"),
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
                    async move { crate::store::tunnels::ssh_host_delete(id, app.state()).await },
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

impl Render for SshHostsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let query = self.search.read(cx).value();
        let cards = store
            .ssh_hosts
            .iter()
            .filter(|item| common::matches(&query, &[&item.name, &item.host, &item.username]))
            .map(|item| {
                let (open, delete, name) = (item.id.clone(), item.id.clone(), item.name.clone());
                common::item_card(
                    &item.id,
                    "server",
                    &item.name,
                    vec![
                        format!("{}@{}:{}", item.username, item.host, item.port).into(),
                        auth_label(item),
                    ],
                    cx.listener(move |this, _, _, cx| this.open(Some(open.clone()), cx)),
                    cx.listener(move |this, _, window, cx| {
                        this.request_delete(delete.clone(), name.clone(), window, cx)
                    }),
                )
                .into_any_element()
            })
            .collect();
        common::list_page(
            t("SSH Hosts"),
            &self.search,
            t("Add host"),
            cx.listener(|this, _, _, cx| this.open(None, cx)),
            self.error.clone(),
            if store.ssh_hosts.is_empty() {
                t("No SSH hosts saved yet")
            } else {
                t("No hosts match your search.")
            },
            cards,
        )
    }
}

// --- Form --------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct Errors {
    name: Option<&'static str>,
    host: Option<&'static str>,
    port: Option<&'static str>,
    username: Option<&'static str>,
    key: Option<&'static str>,
}

/// `port` is the text as typed; `use_key` is the SSH key switch.
fn validate(record: &SshHostRecord, port: &str, use_key: bool) -> Errors {
    Errors {
        name: record.name.is_empty().then_some("Host name is required"),
        host: record.host.is_empty().then_some("Host is required"),
        port: (!matches!(port.trim().parse::<u16>(), Ok(1..)))
            .then_some("Port must be between 1 and 65535"),
        username: record.username.is_empty().then_some("Username is required"),
        key: (use_key && record.key_id.is_none()).then_some("Select a saved key"),
    }
}

pub struct SshHostForm {
    store: Entity<Store>,
    /// The host being edited; `None` adds a new one.
    id: Option<String>,
    /// Editing one that no longer exists.
    missing: bool,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    use_key: bool,
    saved_key: Entity<ChoiceSelect>,
    errors: Errors,
    error: Option<SharedString>,
    saving: bool,
}

impl SshHostForm {
    pub fn new(id: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let (existing, keys) = {
            let store = store.read(cx);
            let existing = id
                .as_deref()
                .and_then(|id| store.ssh_hosts.iter().find(|host| host.id == id))
                .cloned()
                .unwrap_or_else(|| SshHostRecord {
                    port: DEFAULT_PORT,
                    ..Default::default()
                });
            let keys = store
                .ssh_keys
                .iter()
                .map(|key| Choice::new(key.id.clone(), key.name.clone()))
                .collect();
            (existing, keys)
        };
        let saved_key = common::new_select(
            keys,
            existing.key_id.as_deref().unwrap_or_default(),
            window,
            cx,
        );
        let mut input = |value: String, example: &'static str| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder(tf("e.g. {example}", &[("example", example)]))
            })
        };
        Self {
            missing: id.is_some() && existing.id.is_empty(),
            name: input(existing.name, "bastion"),
            host: input(existing.host, "bastion.example.com"),
            port: input(existing.port.to_string(), "22"),
            username: input(existing.username, "ubuntu"),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .default_value(existing.password)
            }),
            use_key: existing.key_id.is_some(),
            saved_key,
            store,
            id,
            errors: Errors::default(),
            error: None,
            saving: false,
        }
    }

    fn record(&self, cx: &App) -> SshHostRecord {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        SshHostRecord {
            id: self.id.clone().unwrap_or_default(),
            name: text(&self.name),
            host: text(&self.host),
            port: text(&self.port).parse().unwrap_or(0),
            username: text(&self.username),
            // A password may begin or end with a space.
            password: self.password.read(cx).value().to_string(),
            key_id: self
                .use_key
                .then(|| common::selected(&self.saved_key, cx))
                .flatten(),
        }
    }

    /// Checks the fields; `false` when the form cannot be saved yet.
    fn check(&mut self, cx: &mut Context<Self>) -> bool {
        self.errors = validate(&self.record(cx), &self.port.read(cx).value(), self.use_key);
        cx.notify();
        self.errors == Errors::default()
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
            let saved =
                backend(
                    async move { crate::store::tunnels::ssh_host_save(record, app.state()).await },
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

impl Render for SshHostForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.missing {
            return common::not_found(
                t("SSH host not found"),
                t("Back to SSH hosts"),
                cx.listener(|this, _, _, cx| this.back(cx)),
            );
        }
        let editing = self.id.is_some();
        let saving = self.saving;
        let submit_label = match (saving, editing) {
            (true, _) => t("Saving…"),
            (false, true) => t("Save changes"),
            (false, false) => t("Save"),
        };
        let store = self.store.read(cx);
        let selected_key = common::selected(&self.saved_key, cx)
            .and_then(|id| store.ssh_keys.iter().find(|key| key.id == id))
            .map(|key| key.name.clone());
        let key_fields = common::field(
            t("Saved key"),
            Select::new(&self.saved_key).disabled(saving),
            self.errors.key.map(t),
        )
        .children(selected_key.map(|name| {
            div()
                .text_xs()
                .text_color(theme::GREEN_TEXT)
                .child(tf("Using saved key: {name}", &[("name", &name)]))
        }))
        .when(store.ssh_keys.is_empty(), |field| {
            field.child(
                div()
                    .text_xs()
                    .text_color(theme::STENCIL_MUTE)
                    .child(t("No SSH keys saved yet")),
            )
        });

        common::form_page(
            self.error.clone(),
            v_flex()
                .gap_5()
                .child(
                    common::form_section(None, None)
                        .child(common::field(
                            t("Name"),
                            Input::new(&self.name).disabled(saving),
                            self.errors.name.map(t),
                        ))
                        .child(
                            h_flex()
                                .items_start()
                                .gap_3()
                                .child(
                                    common::field(
                                        t("Hostname"),
                                        Input::new(&self.host).disabled(saving),
                                        self.errors.host.map(t),
                                    )
                                    .flex_1()
                                    .min_w_0(),
                                )
                                .child(
                                    common::field(
                                        t("Port"),
                                        Input::new(&self.port).disabled(saving),
                                        self.errors.port.map(t),
                                    )
                                    .w(px(112.)),
                                ),
                        )
                        .child(common::field(
                            t("Username"),
                            Input::new(&self.username).disabled(saving),
                            self.errors.username.map(t),
                        )),
                )
                .child(
                    common::form_section(t("Authentication"), None)
                        .child(common::field(
                            t("Password"),
                            Input::new(&self.password).mask_toggle().disabled(saving),
                            None,
                        ))
                        .child(
                            v_flex()
                                .gap_4()
                                .pt_3()
                                .border_t_1()
                                .border_color(theme::hairline())
                                .child(
                                    h_flex()
                                        .justify_between()
                                        .gap_4()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(t("SSH key")),
                                        )
                                        .child(
                                            Switch::new("use-ssh-key")
                                                .accessibility_label(t("Use SSH key"))
                                                .checked(self.use_key)
                                                .disabled(saving)
                                                .on_change(cx.listener(
                                                    |this, checked: &bool, _, cx| {
                                                        this.use_key = *checked;
                                                        this.errors.key = None;
                                                        cx.notify();
                                                    },
                                                )),
                                        ),
                                )
                                .when(self.use_key, |section| section.child(key_fields)),
                        ),
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
    use super::{validate, Errors};
    use shared::SshHostRecord;

    #[test]
    fn a_host_needs_a_name_an_address_a_user_and_the_key_it_asks_for() {
        let host = SshHostRecord {
            name: "bastion".to_string(),
            host: "bastion.example.com".to_string(),
            username: "ubuntu".to_string(),
            ..Default::default()
        };
        assert_eq!(validate(&host, "22", false), Errors::default());
        assert_eq!(
            validate(&SshHostRecord::default(), "0", true),
            Errors {
                name: Some("Host name is required"),
                host: Some("Host is required"),
                port: Some("Port must be between 1 and 65535"),
                username: Some("Username is required"),
                key: Some("Select a saved key"),
            }
        );
    }
}
