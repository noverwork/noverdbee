//! The settings page: general, data, and about, one
//! category at a time.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::bridge::backend;
use crate::ui::i18n::{self, t, tf, Locale};
use crate::ui::icons::lucide;
use crate::ui::store::{store, Store};
use crate::ui::theme;
use crate::ui::views::common::{self, Choice, ChoiceSelect};
use crate::ui::views::updater;

#[derive(Clone, Copy, PartialEq)]
enum Category {
    General,
    Data,
    About,
}

impl Category {
    const ALL: [Category; 3] = [Category::General, Category::Data, Category::About];

    fn label(self) -> SharedString {
        match self {
            Category::General => t("General"),
            Category::Data => t("Data"),
            Category::About => t("About"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Category::General => "languages",
            Category::Data => "database",
            Category::About => "info",
        }
    }
}

#[derive(PartialEq)]
enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Error(String),
}

pub struct SettingsView {
    store: Entity<Store>,
    category: Category,
    language: Entity<ChoiceSelect>,
    update: UpdateStatus,
    _update: Option<Task<()>>,
    backing_up: bool,
    backup_notice: Option<SharedString>,
    backup_error: Option<SharedString>,
    restoring: bool,
    restore_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let languages = Locale::ALL
            .map(|locale| Choice::new(locale.code(), locale.label()))
            .to_vec();
        let language = common::new_select(languages, i18n::locale().code(), window, cx);
        let subscriptions = vec![cx.subscribe(
            &language,
            |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                let SelectEvent::Confirm(value) = event;
                if let Some(locale) = value.as_deref().and_then(Locale::from_code) {
                    i18n::set_locale(locale);
                    crate::ui::set_menus(cx);
                    this.store.update(cx, |store, cx| {
                        store.set_setting(i18n::LOCALE_SETTING, locale.code().to_string(), cx)
                    });
                    cx.refresh_windows();
                }
            },
        )];
        Self {
            store,
            category: Category::General,
            language,
            update: UpdateStatus::Idle,
            _update: None,
            backing_up: false,
            backup_notice: None,
            backup_error: None,
            restoring: false,
            restore_error: None,
            _subscriptions: subscriptions,
        }
    }

    fn check_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update = UpdateStatus::Checking;
        cx.notify();
        let check = updater::check(window, cx);
        self._update = Some(cx.spawn(async move |this, cx| {
            // A found update opens its own dialog; only say "up to date" when
            // there is none.
            let status = match check.await {
                Ok(true) => UpdateStatus::Idle,
                Ok(false) => UpdateStatus::UpToDate,
                Err(error) => UpdateStatus::Error(error),
            };
            let settled = status == UpdateStatus::Idle;
            let _ = this.update(cx, |this, cx| {
                this.update = status;
                cx.notify();
            });
            if !settled {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let _ = this.update(cx, |this, cx| {
                    this.update = UpdateStatus::Idle;
                    cx.notify();
                });
            }
        }));
    }

    fn back_up(&mut self, cx: &mut Context<Self>) {
        self.backing_up = true;
        self.backup_notice = None;
        self.backup_error = None;
        cx.notify();
        let app = self.store.read(cx).app();
        cx.spawn(async move |this, cx| {
            let saved = backend(async move { crate::store::db_backup(app.state()).await }).await;
            let _ = this.update(cx, |this, cx| {
                this.backing_up = false;
                match saved {
                    Ok(file_name) => {
                        this.backup_notice = Some(tf(
                            "Saved to Downloads/{fileName}",
                            &[("fileName", &file_name)],
                        ));
                    }
                    Err(error) => this.backup_error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Picks a backup, checks it, and asks before replacing everything.
    fn choose_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_error = None;
        cx.notify();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("Restore")),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let checked = {
                let path = path.clone();
                backend(async move { crate::store::check_backup(&path) }).await
            };
            let _ = this.update_in(cx, |this, window, cx| match checked {
                Ok(()) => this.confirm_restore(path, window, cx),
                Err(error) => {
                    this.restore_error = Some(error.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn confirm_restore(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        common::confirm_delete(
            t("Restore Backup?"),
            t("Replaces all current data and restarts Noverdbee."),
            Some(name.into()),
            t("Restore and restart"),
            window,
            cx,
            move |_, cx| view.update(cx, |this, cx| this.restore(path.clone(), cx)),
        );
    }

    fn restore(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.restoring = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let staged = backend(crate::store::db_restore(path)).await;
            let _ = this.update(cx, |this, cx| match staged {
                // The next launch swaps the backup in before opening the database.
                Ok(()) => cx.restart(),
                Err(error) => {
                    this.restoring = false;
                    this.restore_error = Some(error.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

fn note(text: SharedString, color: Rgba) -> Div {
    div().text_xs().text_color(color).child(text)
}

/// A titled group of setting rows, with an optional line under the title.
fn group(title: Option<SharedString>, hint: Option<SharedString>, rows: Vec<AnyElement>) -> Div {
    v_flex()
        .gap_3()
        .when(title.is_some() || hint.is_some(), |group| {
            group.child(
                v_flex()
                    .gap_1()
                    .children(title.map(common::section_title))
                    .children(hint.map(|hint| note(hint, theme::STENCIL_MUTE))),
            )
        })
        .child(
            v_flex()
                .rounded(theme::RADIUS)
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::RIB)
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    div()
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(theme::hairline())
                        })
                        .child(row)
                })),
        )
}

/// A label, with an optional explanation, beside its control.
fn row(label: SharedString, hint: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    h_flex()
        .gap_6()
        .px_4()
        .py_3()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().child(label))
                .children(hint.map(|hint| note(hint, theme::STENCIL_MUTE))),
        )
        .child(
            h_flex()
                .w(px(280.))
                .flex_shrink_0()
                .justify_end()
                .child(control),
        )
        .into_any_element()
}

/// Inputs and selects fill the control column; switches and buttons do not.
fn wide(control: impl IntoElement) -> Div {
    div().w_full().child(control)
}

impl SettingsView {
    fn render_nav(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(200.))
            .flex_shrink_0()
            .gap_1()
            .children(Category::ALL.map(|category| {
                let active = category == self.category;
                h_flex()
                    .id(category.icon())
                    .h_9()
                    .gap_3()
                    .px_3()
                    .rounded(theme::RADIUS)
                    .border_1()
                    .text_sm()
                    .cursor_pointer()
                    .map(|item| {
                        if active {
                            item.border_color(theme::hairline())
                                .bg(theme::RIB)
                                .text_color(theme::STENCIL)
                                .font_weight(FontWeight::MEDIUM)
                        } else {
                            item.border_color(gpui_kit::transparent_black())
                                .text_color(theme::STENCIL_MUTE)
                                .hover(|style| style.bg(theme::wash()).text_color(theme::STENCIL))
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.category = category;
                        cx.notify();
                    }))
                    .child(lucide(category.icon()).size_4())
                    .child(category.label())
            }))
    }

    fn render_general(&self) -> Div {
        group(
            None,
            None,
            vec![row(
                t("Language"),
                None,
                wide(Select::new(&self.language).small()),
            )],
        )
    }

    fn render_data(&self, cx: &Context<Self>) -> Div {
        let backup = Button::new("backup")
            .outline()
            .small()
            .map(|button| {
                if self.backing_up {
                    button.label(t("Backing up...")).disabled(true)
                } else {
                    button
                        .icon(lucide("database-backup"))
                        .label(t("Back up to Downloads"))
                }
            })
            .on_click(cx.listener(|this, _, _, cx| this.back_up(cx)));
        let restore = Button::new("restore")
            .outline()
            .small()
            .map(|button| {
                if self.restoring {
                    button.label(t("Restoring...")).disabled(true)
                } else {
                    button
                        .icon(lucide("archive-restore"))
                        .label(t("Choose backup…"))
                }
            })
            .on_click(cx.listener(|this, _, window, cx| this.choose_backup(window, cx)));

        group(
            None,
            None,
            vec![
                row(
                    t("Back up"),
                    Some(t("Stored in plain text — keep it private.")),
                    backup,
                ),
                row(t("Restore from backup"), None, restore),
            ],
        )
        .children(
            self.backup_notice
                .clone()
                .map(|notice| note(notice, theme::GREEN_TEXT)),
        )
        .children(
            self.backup_error
                .clone()
                .map(|error| note(error, theme::RED_TEXT)),
        )
        .children(
            self.restore_error
                .clone()
                .map(|error| note(error, theme::RED_TEXT)),
        )
    }

    fn render_about(&self, cx: &Context<Self>) -> Div {
        let check = Button::new("check-update").outline().small();
        let check = match &self.update {
            UpdateStatus::Checking => check.label(t("Checking...")).disabled(true),
            UpdateStatus::UpToDate => check
                .icon(lucide("circle-check").text_color(theme::GREEN_TEXT))
                .label(t("Up to date"))
                .text_color(theme::GREEN_TEXT),
            _ => check.icon(lucide("download")).label(t("Check for updates")),
        };
        group(
            None,
            None,
            vec![
                row(
                    t("Version"),
                    None,
                    div()
                        .text_sm()
                        .text_color(theme::STENCIL_DIM)
                        .child(env!("CARGO_PKG_VERSION")),
                ),
                row(
                    t("Updates"),
                    None,
                    check
                        .on_click(cx.listener(|this, _, window, cx| this.check_update(window, cx))),
                ),
            ],
        )
        .children(match &self.update {
            UpdateStatus::Error(error) => Some(note(
                format!("{}: {error}", t("Failed to check for updates")).into(),
                theme::RED_TEXT,
            )),
            _ => None,
        })
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.category {
            Category::General => self.render_general(),
            Category::Data => self.render_data(cx),
            Category::About => self.render_about(cx),
        };
        common::page(
            t("Settings"),
            div(),
            h_flex()
                .mt_6()
                .flex_1()
                .min_h_0()
                .items_start()
                .gap_10()
                .child(self.render_nav(cx))
                .child(
                    div()
                        .id("settings")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .child(div().max_w(px(720.)).pb_6().child(content)),
                ),
        )
    }
}
