//! The Databases page: the tab strip, the navigator beside the open tab, and
//! the status line. The shell keeps one for the life of the window, so what is
//! open survives a visit to another page.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::TableRef;
use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::shell::page_label;
use crate::ui::stencil;
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::common;
use crate::ui::views::navigator::{state_shape, ConnectionState, Navigator, NavigatorEvent};
use crate::ui::views::sql_editor::SqlEditor;
use crate::ui::views::table_view::TableView;

actions!(
    noverdbee,
    [
        NewEditor,
        OpenFile,
        CloseTab,
        NextTab,
        PreviousTab,
        FilterTables,
        ReloadTable
    ]
);

/// Shows the tab at this index.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = noverdbee, no_json)]
pub struct SelectTab(pub usize);

pub fn bind_keys(cx: &mut App) {
    // Cmd on macOS; Ctrl elsewhere.
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let key = |keys: &str| -> &'static str { format!("{modifier}-{keys}").leak() };
    let mut bindings = vec![
        KeyBinding::new(key("t"), NewEditor, Some(CONTEXT)),
        KeyBinding::new(key("o"), OpenFile, Some(CONTEXT)),
        KeyBinding::new(key("w"), CloseTab, Some(CONTEXT)),
        KeyBinding::new(key("shift-]"), NextTab, Some(CONTEXT)),
        KeyBinding::new(key("shift-["), PreviousTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some(CONTEXT)),
        KeyBinding::new(key("p"), FilterTables, Some(CONTEXT)),
        KeyBinding::new(key("r"), ReloadTable, Some(CONTEXT)),
    ];
    for index in 0..9 {
        let digit = (index + 1).to_string();
        bindings.push(KeyBinding::new(
            key(&digit),
            SelectTab(index),
            Some(CONTEXT),
        ));
    }
    cx.bind_keys(bindings);
}

const CONTEXT: &str = "Workspace";

enum TabView {
    Editor(Entity<SqlEditor>),
    Table(Entity<TableView>),
}

struct Tab {
    id: usize,
    connection: String,
    title: SharedString,
    view: TabView,
    /// Redraws the strip and the status line when the tab's view changes.
    _observer: Subscription,
}

/// Where a connection points: the file, or `user@host:port/database`.
fn address(record: &shared::DbConnectionRecord) -> String {
    if record.driver == crate::db::DRIVER_POSTGRES {
        format!(
            "{}@{}:{}/{}",
            record.username, record.host, record.port, record.database
        )
    } else {
        record.path.clone()
    }
}

/// How a connection is marked: its `GRP·NAME` code and its group's line colour.
struct Identity {
    code: SharedString,
    line: Rgba,
    /// The line colour is light enough to need dark text on it.
    dark_text: bool,
}

pub struct Workspace {
    store: Entity<Store>,
    navigator: Entity<Navigator>,
    tabs: Vec<Tab>,
    /// Id of the tab shown.
    active: Option<usize>,
    next_tab: usize,
    queries: usize,
    /// Keeps the shortcuts working when no tab holds the focus.
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let navigator = cx.new(|cx| Navigator::new(window, cx));
        let _subscriptions = vec![
            cx.observe(&store, |this, _, cx| {
                // Tabs of a connection that was deleted go with it.
                let store = this.store.read(cx);
                this.tabs
                    .retain(|tab| store.connection(&tab.connection).is_some());
                if this.active_tab().is_none() {
                    this.active = this.tabs.last().map(|tab| tab.id);
                }
                cx.notify();
            }),
            cx.observe(&navigator, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &navigator,
                window,
                |this, _, event: &NavigatorEvent, window, cx| match event {
                    NavigatorEvent::NewEditor { connection } => {
                        this.open_editor(connection.clone(), window, cx);
                    }
                    // A connection that just opened gets an editor to start in.
                    NavigatorEvent::Connected { connection } => {
                        if !this.tabs.iter().any(|tab| &tab.connection == connection) {
                            this.open_editor(connection.clone(), window, cx);
                        }
                    }
                    NavigatorEvent::Failed { name, error } => window.push_notification(
                        Notification::error(error.clone()).title(name.clone()),
                        cx,
                    ),
                    NavigatorEvent::OpenTable { connection, table } => {
                        this.open_table(connection.clone(), table.clone(), window, cx)
                    }
                },
            ),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            store,
            navigator,
            tabs: Vec::new(),
            active: None,
            next_tab: 0,
            queries: 0,
            focus,
            _subscriptions,
        }
    }

    pub fn navigator(&self) -> &Entity<Navigator> {
        &self.navigator
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| Some(tab.id) == self.active)
    }

    /// The editor of the tab shown, if it is one.
    pub fn active_editor(&self) -> Option<Entity<SqlEditor>> {
        self.active_tab().and_then(|tab| match &tab.view {
            TabView::Editor(editor) => Some(editor.clone()),
            TabView::Table(_) => None,
        })
    }

    /// The table view of the tab shown, if it is one.
    pub fn active_table(&self) -> Option<Entity<TableView>> {
        self.active_tab().and_then(|tab| match &tab.view {
            TabView::Table(view) => Some(view.clone()),
            TabView::Editor(_) => None,
        })
    }

    pub fn open_editor(
        &mut self,
        connection: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SqlEditor> {
        let editor = cx.new(|cx| SqlEditor::new(connection.clone(), window, cx));
        self.queries += 1;
        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.push(Tab {
            id,
            connection,
            title: tf("Query {number}", &[("number", &self.queries.to_string())]),
            _observer: cx.observe(&editor, |_, _, cx| cx.notify()),
            view: TabView::Editor(editor.clone()),
        });
        self.show(id, window, cx);
        editor
    }

    /// Shows the tab for `table`, opening one if there is none.
    pub fn open_table(
        &mut self,
        connection: String,
        table: TableRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self.tabs.iter().find(|tab| match &tab.view {
            TabView::Table(view) => tab.connection == connection && view.read(cx).table() == &table,
            TabView::Editor(_) => false,
        });
        if let Some(id) = open.map(|tab| tab.id) {
            // Opening the table that is already in front reloads it.
            if self.active == Some(id) {
                if let Some(view) = self.active_table() {
                    view.update(cx, |view, cx| view.reload(window, cx));
                }
            }
            self.show(id, window, cx);
            return;
        }
        let info = match self.navigator.read(cx).state(&connection) {
            Some(ConnectionState::Live(tables)) => tables.iter().find(|info| info.table == table),
            _ => None,
        };
        let Some(info) = info.cloned() else {
            return;
        };
        let title = SharedString::from(table.label());
        let view = cx.new(|cx| TableView::new(connection.clone(), info, window, cx));
        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.push(Tab {
            id,
            connection,
            title,
            _observer: cx.observe(&view, |_, _, cx| cx.notify()),
            view: TabView::Table(view),
        });
        self.show(id, window, cx);
    }

    /// Shows tab `id` and moves the focus into it: an editor gets the cursor,
    /// a table the workspace. The tab that held the focus may no longer be
    /// drawn, and keys would then reach nothing.
    fn show(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(id);
        match self.active_editor() {
            Some(editor) => editor.focus_handle(cx).focus(window, cx),
            None => self.focus.focus(window, cx),
        }
        cx.notify();
    }

    /// Focused, it keeps the workspace shortcuts working.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Closes the tabs `ids`, asking first when one holds edits that are not
    /// saved.
    fn request_close(&mut self, ids: Vec<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.tabs.iter().any(|tab| {
            ids.contains(&tab.id)
                && matches!(&tab.view, TabView::Table(view) if view.read(cx).pending(cx) > 0)
        });
        if !pending {
            self.close(&ids, window, cx);
            return;
        }
        let workspace = cx.weak_entity();
        common::confirm_delete(
            t("Discard changes?"),
            t("Closing the tab drops the changes that have not been saved."),
            None,
            t("Discard"),
            window,
            cx,
            move |window, cx| {
                let _ = workspace.update(cx, |this, cx| this.close(&ids, window, cx));
            },
        );
    }

    fn close(&mut self, ids: &[usize], window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.tabs.iter().position(|tab| Some(tab.id) == self.active);
        self.tabs.retain(|tab| !ids.contains(&tab.id));
        if let Some(index) = shown.filter(|_| self.active_tab().is_none()) {
            // The closed tab held the focus; the one now in its place takes it.
            match self.tabs.get(index.min(self.tabs.len().saturating_sub(1))) {
                Some(tab) => self.show(tab.id, window, cx),
                None => {
                    self.active = None;
                    self.focus.focus(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Opens `.sql` files, each in a new editor on the current connection.
    fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.editor_target(cx) else {
            return;
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t("Open")),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            for path in paths {
                let source = path.clone();
                let read = backend(async move {
                    tokio::fs::read_to_string(&source)
                        .await
                        .map_err(|error| format!("cannot read {}: {error}", source.display()))
                })
                .await;
                let _ = this.update_in(cx, |this, window, cx| match read {
                    Ok(text) => {
                        let editor = this.open_editor(connection.clone(), window, cx);
                        editor.update(cx, |editor, cx| editor.open_file(path, &text, window, cx));
                    }
                    Err(error) => window.push_notification(Notification::error(error), cx),
                });
            }
        })
        .detach();
    }

    /// Shows the tab `step` places after the one shown, wrapping around.
    fn cycle(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| Some(tab.id) == self.active) else {
            return;
        };
        let next = (index as isize + step).rem_euclid(self.tabs.len() as isize) as usize;
        self.show(self.tabs[next].id, window, cx);
    }

    /// Where the `+` button opens an editor: the connection of the tab shown,
    /// else the first one that is open.
    fn editor_target(&self, cx: &App) -> Option<String> {
        if let Some(tab) = self.active_tab() {
            return Some(tab.connection.clone());
        }
        let navigator = self.navigator.read(cx);
        let store = self.store.read(cx);
        store
            .connections
            .iter()
            .find(|row| matches!(navigator.state(&row.id), Some(ConnectionState::Live(_))))
            .map(|row| row.id.clone())
    }

    fn identity(&self, connection: &str, cx: &App) -> Identity {
        let store = self.store.read(cx);
        let record = store.connection(connection);
        let environment = record.map(|record| record.environment).unwrap_or_default();
        let (line, dark_text) = stencil::line_color(environment);
        let name = record.map_or("", |record| record.name.as_str());
        Identity {
            code: format!("{}·{}", stencil::code(environment), name.to_uppercase()).into(),
            line,
            dark_text,
        }
    }

    fn render_tab(&self, tab: &Tab, cx: &Context<Self>) -> impl IntoElement {
        let id = tab.id;
        let active = self.active == Some(id);
        let identity = self.identity(&tab.connection, cx);
        let ink = if active && identity.dark_text {
            theme::PORT
        } else {
            theme::STENCIL
        };
        let (running, pending) = match &tab.view {
            TabView::Editor(editor) => (editor.read(cx).is_running(), false),
            TabView::Table(view) => {
                let view = view.read(cx);
                (view.is_busy(), view.pending(cx) > 0)
            }
        };
        let transaction = self.store.read(cx).in_transaction(&tab.connection);
        let read_only = self
            .store
            .read(cx)
            .connection(&tab.connection)
            .is_some_and(|record| record.read_only);
        let ids: Vec<usize> = self.tabs.iter().map(|tab| tab.id).collect();
        let index = ids.iter().position(|tab| *tab == id).unwrap_or(0);
        let others: Vec<usize> = ids.iter().copied().filter(|tab| *tab != id).collect();
        let workspace = cx.weak_entity();
        let state = if running {
            Some(ConnectionState::Connecting)
        } else {
            self.navigator.read(cx).state(&tab.connection).cloned()
        };

        div()
            .id(("tab", id))
            .h_7()
            .flex_1()
            .min_w(px(176.))
            .max_w(px(240.))
            .flex()
            .items_center()
            .gap_2()
            .pl_2p5()
            .pr_1()
            .rounded(theme::RADIUS)
            .border_1()
            .text_color(ink)
            .cursor_pointer()
            .map(|tab| {
                if active {
                    tab.bg(identity.line).border_color(theme::STENCIL)
                } else {
                    // Mixed into the strip, not faded, so the text keeps its contrast.
                    tab.bg(theme::mix(identity.line, theme::RIB, 0.55))
                        .border_color(theme::hairline())
                        .hover(|style| style.border_color(theme::STENCIL.opacity(0.25)))
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.tabs.iter().any(|tab| tab.id == id) {
                    this.show(id, window, cx);
                }
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .child(identity.code),
            )
            // Unsaved edits wait behind an ochre square.
            .when(pending, |tab| {
                tab.child(div().size_1p5().flex_shrink_0().bg(theme::AMBER_TEXT))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .child(match &tab.view {
                        TabView::Editor(editor) => {
                            editor.read(cx).file_name().unwrap_or(tab.title.clone())
                        }
                        TabView::Table(_) => tab.title.clone(),
                    }),
            )
            .when(read_only, |tab| {
                tab.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.))
                        .text_color(ink.opacity(0.7))
                        .child("RO"),
                )
            })
            .when(transaction, |tab| {
                tab.child(
                    div()
                        .flex_shrink_0()
                        .px_1()
                        .rounded(theme::RADIUS)
                        .bg(theme::PORT)
                        .text_size(px(10.))
                        .text_color(theme::AMBER_TEXT)
                        .child("TX"),
                )
            })
            .child(state_shape(state.as_ref(), ink))
            .child(
                Button::new(("close-tab", id))
                    .ghost()
                    .xsmall()
                    .icon(lucide("x"))
                    .tooltip(t("Close tab"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.request_close(vec![id], window, cx)
                    })),
            )
            .context_menu(move |menu, _, _| {
                let close = |label: SharedString, targets: Vec<usize>| {
                    let workspace = workspace.clone();
                    PopupMenuItem::new(label)
                        .disabled(targets.is_empty())
                        .on_click(move |_, window, cx| {
                            let targets = targets.clone();
                            let _ = workspace
                                .update(cx, |this, cx| this.request_close(targets, window, cx));
                        })
                };
                menu.item(close(t("Close"), vec![id]))
                    .item(close(t("Close Other Tabs"), others.clone()))
                    .item(close(t("Close Tabs to the Left"), ids[..index].to_vec()))
                    .item(close(
                        t("Close Tabs to the Right"),
                        ids[index + 1..].to_vec(),
                    ))
                    .separator()
                    .item(close(t("Close All Tabs"), ids.clone()))
            })
    }

    fn render_strip(&self, cx: &Context<Self>) -> impl IntoElement {
        let target = self.editor_target(cx);
        let has_target = target.is_some();
        h_flex()
            .h_9()
            .flex_shrink_0()
            .gap_1()
            .pr_2()
            .bg(theme::RIB)
            .border_b_1()
            .border_color(theme::RIB)
            .child(page_label(t("Databases"), "database"))
            .child(
                h_flex()
                    .id("tabs")
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    // With the strip's gap, the first tab sits 6px from the cell, as in noverterm.
                    .pl_0p5()
                    .overflow_x_scroll()
                    .children(self.tabs.iter().map(|tab| self.render_tab(tab, cx))),
            )
            .child(
                Button::new("new-editor")
                    .ghost()
                    .small()
                    .icon(lucide("plus"))
                    .tooltip(t("New SQL editor"))
                    .disabled(target.is_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(connection) = target.clone() {
                            this.open_editor(connection, window, cx);
                        }
                    })),
            )
            .child(
                Button::new("open-file")
                    .ghost()
                    .small()
                    .icon(lucide("folder-open"))
                    .tooltip(t("Open a .sql file (Cmd+O)"))
                    .disabled(!has_target)
                    .on_click(cx.listener(|this, _, window, cx| this.open_file(window, cx))),
            )
    }

    fn render_status(&self, cx: &Context<Self>) -> impl IntoElement {
        let navigator = self.navigator.read(cx);
        let (open, held) = navigator.counts();
        let counts = if held > 0 {
            tf(
                "{open} open · {held} held",
                &[("open", &open.to_string()), ("held", &held.to_string())],
            )
        } else {
            tf("{open} open", &[("open", &open.to_string())])
        };
        let store = self.store.read(cx);
        let active = self.active_tab().map(|tab| {
            let identity = self.identity(&tab.connection, cx);
            let path = self
                .store
                .read(cx)
                .connection(&tab.connection)
                .map(|record| SharedString::from(address(record)))
                .unwrap_or_default();
            let (state, held) = match navigator.state(&tab.connection) {
                Some(ConnectionState::Live(_)) => (t("Connected"), false),
                Some(ConnectionState::Connecting) => (t("Connecting"), false),
                Some(ConnectionState::Failed(_)) => (t("Disconnected"), true),
                None => (t("Not connected"), false),
            };
            let summary = match &tab.view {
                TabView::Editor(editor) => editor.read(cx).summary(),
                TabView::Table(view) => view.read(cx).summary(cx),
            };
            h_flex()
                .min_w_0()
                .gap_5()
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .gap_2()
                        .text_color(theme::STENCIL)
                        .child(div().w_2().h_3().bg(identity.line))
                        .child(identity.code),
                )
                .child(div().min_w_0().truncate().child(path))
                .child(
                    div()
                        .flex_shrink_0()
                        .when(held, |state| state.text_color(theme::RED_TEXT))
                        .child(state),
                )
                .when(store.in_transaction(&tab.connection), |line| {
                    line.child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::AMBER_TEXT)
                            .child(t("Transaction open")),
                    )
                })
                .children(summary.map(|summary| div().flex_shrink_0().child(summary)))
        });

        h_flex()
            .h_6()
            .flex_shrink_0()
            .justify_between()
            .gap_5()
            .px_3()
            .bg(theme::RAIL)
            .border_t_1()
            .border_color(theme::RIB)
            .text_size(px(11.))
            .text_color(theme::STENCIL_MUTE)
            .child(active.unwrap_or_else(h_flex))
            .child(div().flex_shrink_0().child(counts))
    }

    /// Shown in front of any open tab while a connection opens: opening can
    /// take seconds, and the user asked for this connection, not that tab.
    fn render_connecting(&self, id: &str, cx: &Context<Self>) -> AnyElement {
        let name = self
            .store
            .read(cx)
            .connection(id)
            .map(|record| SharedString::from(record.name.clone()))
            .unwrap_or_default();
        let (navigator, id) = (self.navigator.clone(), id.to_string());
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(theme::PORT)
            .text_sm()
            .text_color(theme::STENCIL_MUTE)
            .child(Spinner::new().large().color(theme::STENCIL.into()))
            .child(t("Connecting"))
            .child(name)
            .child(
                Button::new("cancel-connect")
                    .ghost()
                    .label(t("Cancel"))
                    .on_click(move |_, _, cx| {
                        navigator.update(cx, |navigator, cx| navigator.disconnect(&id, cx));
                    }),
            )
            .into_any_element()
    }

    fn render_empty(&self, cx: &Context<Self>) -> AnyElement {
        let has_connections = !self.store.read(cx).connections.is_empty();
        let store = self.store.clone();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(theme::PORT)
            .text_sm()
            .text_color(theme::STENCIL_MUTE)
            .child(if has_connections {
                t("Open a connection in the navigator.")
            } else {
                t("No connections yet")
            })
            .children((!has_connections).then(|| {
                Button::new("first-connection")
                    .primary()
                    .label(t("New connection"))
                    .on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| {
                            store.navigate(Route::ConnectionForm(None), cx)
                        });
                    })
            }))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connecting = self.navigator.read(cx).connecting().map(str::to_string);
        let content = match self.active_tab().map(|tab| &tab.view) {
            _ if connecting.is_some() => {
                self.render_connecting(connecting.as_deref().unwrap_or_default(), cx)
            }
            Some(TabView::Editor(editor)) => editor.clone().into_any_element(),
            Some(TabView::Table(view)) => view.clone().into_any_element(),
            None => self.render_empty(cx),
        };
        v_flex()
            .size_full()
            .id("workspace")
            .role(Role::Group)
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NewEditor, window, cx| {
                if let Some(connection) = this.editor_target(cx) {
                    this.open_editor(connection, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.open_file(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if let Some(id) = this.active {
                    this.request_close(vec![id], window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, action: &SelectTab, window, cx| {
                if let Some(id) = this.tabs.get(action.0).map(|tab| tab.id) {
                    this.show(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ReloadTable, window, cx| {
                if let Some(view) = this.active_table() {
                    view.update(cx, |view, cx| view.reload(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FilterTables, window, cx| {
                this.navigator
                    .update(cx, |navigator, cx| navigator.focus_filter(window, cx));
            }))
            .child(self.render_strip(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("workspace")
                        .child(
                            resizable_panel()
                                .size(px(240.))
                                .size_range(px(200.)..px(400.))
                                .child(self.navigator.clone()),
                        )
                        .child(resizable_panel().child(content)),
                ),
            )
            .child(self.render_status(cx))
    }
}
