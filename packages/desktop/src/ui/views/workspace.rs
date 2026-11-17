//! The Databases page: the tab strip, the navigator beside the open tab, and
//! the status line. The shell keeps one for the life of the window, so what is
//! open survives a visit to another page.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::TableRef;
use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::shell::page_label;
use crate::ui::stencil;
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::navigator::{state_shape, ConnectionState, Navigator, NavigatorEvent};
use crate::ui::views::sql_editor::SqlEditor;
use crate::ui::views::table_view::TableView;

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
                        this.open_editor(connection.clone(), window, cx)
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
        Self {
            store,
            navigator,
            tabs: Vec::new(),
            active: None,
            next_tab: 0,
            queries: 0,
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

    pub fn open_editor(&mut self, connection: String, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(|cx| SqlEditor::new(connection.clone(), window, cx));
        self.queries += 1;
        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.push(Tab {
            id,
            connection,
            title: tf("Query {number}", &[("number", &self.queries.to_string())]),
            _observer: cx.observe(&editor, |_, _, cx| cx.notify()),
            view: TabView::Editor(editor),
        });
        self.active = Some(id);
        cx.notify();
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
        if let Some(tab) = open {
            self.active = Some(tab.id);
            cx.notify();
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
        self.active = Some(id);
        cx.notify();
    }

    fn close(&mut self, id: usize, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active == Some(id) {
            let neighbour = index.min(self.tabs.len().saturating_sub(1));
            self.active = self.tabs.get(neighbour).map(|tab| tab.id);
        }
        cx.notify();
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
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.tabs.iter().any(|tab| tab.id == id) {
                    this.active = Some(id);
                    cx.notify();
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
                    .child(tab.title.clone()),
            )
            .child(state_shape(state.as_ref(), ink))
            .child(
                Button::new(("close-tab", id))
                    .ghost()
                    .xsmall()
                    .icon(lucide("x"))
                    .tooltip(t("Close tab"))
                    .on_click(cx.listener(move |this, _, _, cx| this.close(id, cx))),
            )
    }

    fn render_strip(&self, cx: &Context<Self>) -> impl IntoElement {
        let target = self.editor_target(cx);
        h_flex()
            .h_9()
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .bg(theme::RIB)
            .child(page_label(t("Databases"), "database"))
            .child(
                h_flex()
                    .id("tabs")
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .pl_1()
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

    fn render_empty(&self, cx: &Context<Self>) -> impl IntoElement {
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
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.active_tab().map(|tab| &tab.view) {
            Some(TabView::Editor(editor)) => editor.clone().into_any_element(),
            Some(TabView::Table(view)) => view.clone().into_any_element(),
            None => self.render_empty(cx).into_any_element(),
        };
        v_flex()
            .size_full()
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
