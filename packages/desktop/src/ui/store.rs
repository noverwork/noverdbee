//! App-wide UI state: saved data and the current page.

use std::collections::HashSet;

use gpui_kit::*;
use shared::{
    DbConnectionRecord, GroupRecord, KubeconfigRecord, Setting, SshHostRecord, SshKeyRecord,
};

use crate::app::AppHandle;
use crate::ui::bridge::backend;
use crate::ui::i18n::{self, Locale};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Databases,
    /// Add a connection, or edit the one with this id.
    ConnectionForm(Option<String>),
    /// Add a connection to the group with this name.
    NewConnectionIn(String),
    SshHosts,
    /// Add an SSH host, or edit the one with this id.
    SshHostForm(Option<String>),
    SshKeys,
    /// Add an SSH key, or edit the one with this id.
    SshKeyForm(Option<String>),
    Kubeconfigs,
    /// Add a kubeconfig, or edit the one with this id.
    KubeconfigForm(Option<String>),
    Mcp,
    Settings,
}

impl Route {
    /// Forms open in a drawer over the current page instead of replacing it.
    pub fn is_form(&self) -> bool {
        matches!(
            self,
            Route::ConnectionForm(_)
                | Route::NewConnectionIn(_)
                | Route::SshHostForm(_)
                | Route::SshKeyForm(_)
                | Route::KubeconfigForm(_)
        )
    }
}

pub struct Store {
    app: AppHandle,
    pub route: Route,
    /// The form open in the drawer, if any.
    pub form: Option<Route>,
    pub loading: bool,
    pub load_error: Option<String>,
    pub groups: Vec<GroupRecord>,
    pub connections: Vec<DbConnectionRecord>,
    pub ssh_hosts: Vec<SshHostRecord>,
    pub ssh_keys: Vec<SshKeyRecord>,
    pub kubeconfigs: Vec<KubeconfigRecord>,
    pub settings: Vec<Setting>,
    /// Connections whose session has a transaction open, as last checked.
    pub transactions: HashSet<String>,
}

/// The store, reachable from any view.
pub struct Globals {
    pub store: Entity<Store>,
}

impl Global for Globals {}

pub fn store(cx: &App) -> Entity<Store> {
    cx.global::<Globals>().store.clone()
}

impl Store {
    pub fn new(app: AppHandle, cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            app,
            route: Route::Databases,
            form: None,
            loading: true,
            load_error: None,
            groups: Vec::new(),
            connections: Vec::new(),
            ssh_hosts: Vec::new(),
            ssh_keys: Vec::new(),
            kubeconfigs: Vec::new(),
            settings: Vec::new(),
            transactions: HashSet::new(),
        };
        store.refresh(cx);
        store
    }

    pub fn app(&self) -> AppHandle {
        self.app.clone()
    }

    /// Shows a page, or opens a form in the drawer over the current page.
    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        if route.is_form() {
            self.form = Some(route);
        } else {
            self.route = route;
            self.form = None;
        }
        cx.notify();
    }

    pub fn close_form(&mut self, cx: &mut Context<Self>) {
        if self.form.take().is_some() {
            cx.notify();
        }
    }

    // --- Saved data ---------------------------------------------------------

    /// Reloads everything saved in the database.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let app = self.app.clone();
        cx.spawn(async move |this, cx| {
            let loaded = backend(async move {
                Ok::<_, String>((
                    crate::store::settings::get_all_settings(app.state()).await?,
                    crate::store::groups::group_list(app.state()).await?,
                    crate::store::connections::connection_list(app.state()).await?,
                    crate::store::tunnels::ssh_host_list(app.state()).await?,
                    crate::store::tunnels::ssh_key_list(app.state()).await?,
                    crate::store::tunnels::kubeconfig_list(app.state()).await?,
                ))
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                match loaded {
                    Ok((settings, groups, connections, ssh_hosts, ssh_keys, kubeconfigs)) => {
                        let first_load = this.loading;
                        this.settings = settings;
                        this.groups = groups;
                        this.connections = connections;
                        this.ssh_hosts = ssh_hosts;
                        this.ssh_keys = ssh_keys;
                        this.kubeconfigs = kubeconfigs;
                        this.load_error = None;
                        if first_load {
                            let saved = this.setting(i18n::LOCALE_SETTING);
                            i18n::set_locale(
                                saved
                                    .and_then(Locale::from_code)
                                    .unwrap_or_else(Locale::system),
                            );
                            crate::ui::set_menus(cx);
                        }
                    }
                    Err(error) => this.load_error = Some(error),
                }
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn in_transaction(&self, connection: &str) -> bool {
        self.transactions.contains(connection)
    }

    /// Records whether `connection` has a transaction open.
    pub fn set_transaction(&mut self, connection: &str, open: bool, cx: &mut Context<Self>) {
        let changed = if open {
            self.transactions.insert(connection.to_string())
        } else {
            self.transactions.remove(connection)
        };
        if changed {
            cx.notify();
        }
    }

    pub fn connection(&self, id: &str) -> Option<&DbConnectionRecord> {
        self.connections
            .iter()
            .find(|connection| connection.id == id)
    }

    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings
            .iter()
            .find(|setting| setting.key == key)
            .map(|setting| setting.value.as_str())
    }

    /// Saves one setting, then reloads.
    pub fn set_setting(&mut self, key: &str, value: String, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let setting = Setting {
            key: key.to_string(),
            value,
        };
        // Reflect it right away; the reload confirms it.
        match self
            .settings
            .iter_mut()
            .find(|saved| saved.key == setting.key)
        {
            Some(saved) => saved.value = setting.value.clone(),
            None => self.settings.push(setting.clone()),
        }
        cx.spawn(async move |this, cx| {
            let saved =
                backend(
                    async move { crate::store::settings::set_setting(setting, app.state()).await },
                )
                .await;
            if let Err(error) = saved {
                tracing::warn!(error, "Failed to save setting");
            }
            let _ = this.update(cx, |this, cx| this.refresh(cx));
        })
        .detach();
        cx.notify();
    }
}
