//! App-wide UI state: saved data and the current page.

use gpui_kit::*;
use shared::{DbConnectionRecord, GroupRecord, Setting};

use crate::app::AppHandle;
use crate::ui::bridge::backend;
use crate::ui::i18n::{self, Locale};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Databases,
    /// Add a connection, or edit the one with this id.
    ConnectionForm(Option<String>),
    Settings,
}

pub struct Store {
    app: AppHandle,
    pub route: Route,
    pub loading: bool,
    pub load_error: Option<String>,
    pub groups: Vec<GroupRecord>,
    pub connections: Vec<DbConnectionRecord>,
    pub settings: Vec<Setting>,
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
            loading: true,
            load_error: None,
            groups: Vec::new(),
            connections: Vec::new(),
            settings: Vec::new(),
        };
        store.refresh(cx);
        store
    }

    pub fn app(&self) -> AppHandle {
        self.app.clone()
    }

    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.route = route;
        cx.notify();
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
                ))
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                match loaded {
                    Ok((settings, groups, connections)) => {
                        let first_load = this.loading;
                        this.settings = settings;
                        this.groups = groups;
                        this.connections = connections;
                        this.load_error = None;
                        if first_load {
                            let saved = this.setting(i18n::LOCALE_SETTING);
                            i18n::set_locale(
                                saved
                                    .and_then(Locale::from_code)
                                    .unwrap_or_else(Locale::system),
                            );
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
