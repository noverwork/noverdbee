//! The object tree: groups, their connections, and the tables and columns of
//! each connection that is open. It owns which connections are open.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::tree::{tree, TreeEntry, TreeEvent, TreeItem, TreeState};
use gpui_kit::component::{h_flex, v_flex, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use shared::DbConnectionRecord;

use crate::db::{ObjectKind, SessionManager, TableInfo, TableRef};
use crate::ui::bridge::backend;
use crate::ui::i18n::t;
use crate::ui::icons::lucide;
use crate::ui::stencil;
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::common;

/// Where a saved connection stands.
#[derive(Clone)]
pub enum ConnectionState {
    Connecting,
    Live(Rc<Vec<TableInfo>>),
    Failed(SharedString),
}

pub enum NavigatorEvent {
    /// A connection finished opening.
    Connected { connection: String },
    /// A connection could not be opened.
    Failed { name: String, error: String },
    /// The user asked for a SQL editor on a connection.
    NewEditor { connection: String },
    /// The user asked to see a table's rows.
    OpenTable { connection: String, table: TableRef },
}

/// What a tree row stands for, by tree item id.
#[derive(Clone)]
enum Node {
    /// A folder of connections; it carries no colour.
    Group {
        id: String,
        name: SharedString,
        count: usize,
    },
    Connection {
        id: String,
        state: Option<ConnectionState>,
        /// The environment's line colour.
        line: Rgba,
    },
    Table {
        connection: String,
        table: TableRef,
        kind: ObjectKind,
    },
    Column {
        type_name: SharedString,
        primary_key: bool,
    },
    /// A line of text under a connection: "no tables".
    Note,
}

pub struct Navigator {
    store: Entity<Store>,
    tree: Entity<TreeState>,
    filter: Entity<InputState>,
    states: HashMap<String, ConnectionState>,
    /// Rows the user opened or closed; the rest keep their default.
    expanded: HashMap<SharedString, bool>,
    nodes: Rc<HashMap<SharedString, Node>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<NavigatorEvent> for Navigator {}

impl Navigator {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let tree = cx.new(|cx| TreeState::new(cx));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("Filter tables")));
        let _subscriptions = vec![
            cx.observe(&store, |this, _, cx| this.reconcile(cx)),
            cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild(cx);
                }
            }),
            cx.subscribe(&tree, |this, _, event: &TreeEvent, _| match event {
                TreeEvent::Expanded(id) => {
                    this.expanded.insert(id.clone(), true);
                }
                TreeEvent::Collapsed(id) => {
                    this.expanded.insert(id.clone(), false);
                }
            }),
        ];
        let mut navigator = Self {
            store,
            tree,
            filter,
            states: HashMap::new(),
            expanded: HashMap::new(),
            nodes: Rc::default(),
            _subscriptions,
        };
        navigator.rebuild(cx);
        navigator
    }

    pub fn state(&self, connection: &str) -> Option<&ConnectionState> {
        self.states.get(connection)
    }

    /// How many connections are open, and how many failed to open.
    pub fn counts(&self) -> (usize, usize) {
        let count = |wanted: fn(&ConnectionState) -> bool| {
            self.states.values().filter(|state| wanted(state)).count()
        };
        (
            count(|state| matches!(state, ConnectionState::Live(_))),
            count(|state| matches!(state, ConnectionState::Failed(_))),
        )
    }

    /// Opens the connection and loads its tables.
    pub fn connect(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(record) = self.store.read(cx).connection(&id).cloned() else {
            return;
        };
        self.states.insert(id.clone(), ConnectionState::Connecting);
        self.rebuild(cx);
        let app = self.store.read(cx).app();
        let name = record.name.clone();
        cx.spawn(async move |this, cx| {
            let loaded = backend(async move {
                let session = app.state::<SessionManager>().connect(&record).await?;
                session.tables().await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                let state = match loaded {
                    Ok(tables) => {
                        cx.emit(NavigatorEvent::Connected {
                            connection: id.clone(),
                        });
                        ConnectionState::Live(Rc::new(tables))
                    }
                    Err(error) => {
                        cx.emit(NavigatorEvent::Failed {
                            name,
                            error: error.clone(),
                        });
                        ConnectionState::Failed(error.into())
                    }
                };
                this.states.insert(id, state);
                this.rebuild(cx);
            });
        })
        .detach();
    }

    pub fn disconnect(&mut self, id: &str, cx: &mut Context<Self>) {
        self.store
            .read(cx)
            .app()
            .state::<SessionManager>()
            .disconnect(id);
        self.states.remove(id);
        self.rebuild(cx);
    }

    fn delete(&mut self, record: DbConnectionRecord, window: &mut Window, cx: &mut Context<Self>) {
        let navigator = cx.weak_entity();
        let id = record.id;
        common::confirm_delete(
            t("Delete connection?"),
            t("The saved connection is removed. The database itself is not touched."),
            Some(record.name.into()),
            t("Delete"),
            window,
            cx,
            move |_, cx| {
                let id = id.clone();
                let _ = navigator.update(cx, |this, cx| {
                    this.disconnect(&id, cx);
                    let app = this.store.read(cx).app();
                    let store = this.store.clone();
                    cx.spawn(async move |_, cx| {
                        let deleted = backend(async move {
                            crate::store::connections::connection_delete(id, app.state()).await
                        })
                        .await;
                        if let Err(error) = deleted {
                            tracing::warn!(error, "Failed to delete connection");
                        }
                        store.update(cx, |store, cx| store.refresh(cx));
                    })
                    .detach();
                });
            },
        );
    }

    /// Asks for a group's new name, then renames it.
    fn rename_group(
        &mut self,
        id: String,
        name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |input, cx| input.focus(window, cx));
        let navigator = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (navigator, input, id) = (navigator.clone(), input.clone(), id.clone());
            alert
                .title(t("Rename group"))
                .description(Input::new(&input))
                .confirm()
                .ok_text(t("Rename"))
                .cancel_text(t("Cancel"))
                .on_ok(move |_, window, cx| {
                    let name = input.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        return false;
                    }
                    let id = id.clone();
                    let _ = navigator.update(cx, |this, cx| {
                        let app = this.store.read(cx).app();
                        this.change_groups(
                            async move {
                                crate::store::groups::group_rename(id, name, app.state()).await
                            },
                            window,
                            cx,
                        )
                    });
                    true
                })
        });
    }

    /// Takes every connection out of a group.
    fn dissolve_group(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let app = self.store.read(cx).app();
        self.change_groups(
            async move { crate::store::groups::group_dissolve(id, app.state()).await },
            window,
            cx,
        );
    }

    /// Runs a group change in the backend, then reloads; a failure is a toast.
    fn change_groups(
        &mut self,
        change: impl std::future::Future<Output = Result<(), String>> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        cx.spawn_in(window, async move |_, cx| {
            let changed = backend(change).await;
            let _ = cx.update(|window, cx| {
                if let Err(error) = changed {
                    window.push_notification(Notification::error(error), cx);
                }
                store.update(cx, |store, cx| store.refresh(cx));
            });
        })
        .detach();
    }

    /// Drops the state of connections that were edited or deleted, then redraws.
    fn reconcile(&mut self, cx: &mut Context<Self>) {
        let app = self.store.read(cx).app();
        let sessions = app.state::<SessionManager>();
        let store = self.store.read(cx);
        self.states.retain(|id, state| {
            store.connection(id).is_some()
                && (!matches!(state, ConnectionState::Live(_)) || sessions.get(id).is_some())
        });
        self.rebuild(cx);
    }

    fn is_expanded(&self, id: &SharedString, default: bool) -> bool {
        self.expanded.get(id).copied().unwrap_or(default)
    }

    /// Rebuilds the tree from the saved connections and what is open.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).value().trim().to_lowercase();
        let store = self.store.read(cx);
        let mut nodes = HashMap::new();
        let mut items = Vec::new();

        let ungrouped = store
            .connections
            .iter()
            .filter(|connection| connection.group_id.is_none());
        for connection in ungrouped {
            items.push(self.connection_item(connection, &filter, &mut nodes));
        }
        for group in &store.groups {
            let members: Vec<&DbConnectionRecord> = store
                .connections
                .iter()
                .filter(|connection| connection.group_id.as_deref() == Some(group.id.as_str()))
                .collect();
            let id = SharedString::from(format!("group:{}", group.id));
            nodes.insert(
                id.clone(),
                Node::Group {
                    id: group.id.clone(),
                    name: group.name.clone().into(),
                    count: members.len(),
                },
            );
            let children: Vec<TreeItem> = members
                .into_iter()
                .map(|connection| self.connection_item(connection, &filter, &mut nodes))
                .collect();
            items.push(
                TreeItem::new(id.clone(), group.name.clone())
                    .children(children)
                    .expanded(self.is_expanded(&id, true)),
            );
        }

        self.nodes = Rc::new(nodes);
        self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        cx.notify();
    }

    fn connection_item(
        &self,
        connection: &DbConnectionRecord,
        filter: &str,
        nodes: &mut HashMap<SharedString, Node>,
    ) -> TreeItem {
        let id = SharedString::from(format!("connection:{}", connection.id));
        let state = self.states.get(&connection.id).cloned();
        let mut children = Vec::new();
        if let Some(ConnectionState::Live(tables)) = &state {
            if tables.is_empty() {
                let note_id = SharedString::from(format!("note:{}", connection.id));
                nodes.insert(note_id.clone(), Node::Note);
                children.push(TreeItem::new(note_id, t("No tables")));
            }
        }
        if let Some(ConnectionState::Live(tables)) = &state {
            for table in tables.iter() {
                if !filter.is_empty() && !table.table.label().to_lowercase().contains(filter) {
                    continue;
                }
                children.push(table_item(&connection.id, table, &self.expanded, nodes));
            }
        }
        nodes.insert(
            id.clone(),
            Node::Connection {
                id: connection.id.clone(),
                state,
                line: stencil::line_color(connection.environment).0,
            },
        );
        TreeItem::new(id.clone(), connection.name.clone())
            .children(children)
            .expanded(self.is_expanded(&id, true))
    }

    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let store = self.store.clone();
        h_flex()
            .flex_shrink_0()
            .gap_1()
            .p_2()
            .border_b_1()
            .border_color(theme::RIB)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.filter).small().cleanable(true)),
            )
            .child(
                Button::new("new-connection")
                    .ghost()
                    .small()
                    .icon(lucide("plus"))
                    .tooltip(t("New connection"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        store.update(cx, |store, cx| {
                            store.navigate(Route::ConnectionForm(None), cx)
                        });
                    })),
            )
    }
}

/// The menu of a group row: add a connection to it, rename it, or dissolve it.
fn group_menu(
    menu: PopupMenu,
    navigator: WeakEntity<Navigator>,
    store: Entity<Store>,
    id: &str,
    name: &SharedString,
) -> PopupMenu {
    let new_connection = {
        let name = name.to_string();
        PopupMenuItem::new(t("New connection here")).on_click(move |_, _, cx| {
            store.update(cx, |store, cx| {
                store.navigate(Route::NewConnectionIn(name.clone()), cx)
            });
        })
    };
    let rename = {
        let (navigator, id, name) = (navigator.clone(), id.to_string(), name.clone());
        PopupMenuItem::new(t("Rename")).on_click(move |_, window, cx| {
            let _ = navigator.update(cx, |this, cx| {
                this.rename_group(id.clone(), name.clone(), window, cx)
            });
        })
    };
    let dissolve = {
        let id = id.to_string();
        PopupMenuItem::new(t("Ungroup")).on_click(move |_, window, cx| {
            let _ = navigator.update(cx, |this, cx| this.dissolve_group(id.clone(), window, cx));
        })
    };
    menu.item(new_connection)
        .separator()
        .item(rename)
        .item(dissolve)
}

fn table_item(
    connection: &str,
    table: &TableInfo,
    expanded: &HashMap<SharedString, bool>,
    nodes: &mut HashMap<SharedString, Node>,
) -> TreeItem {
    let id = SharedString::from(format!("table:{connection}:{}", table.table.quoted()));
    nodes.insert(
        id.clone(),
        Node::Table {
            connection: connection.to_string(),
            table: table.table.clone(),
            kind: table.kind,
        },
    );
    let columns = table.columns.iter().map(|column| {
        let column_id = SharedString::from(format!("{id}:{}", column.name));
        nodes.insert(
            column_id.clone(),
            Node::Column {
                type_name: column.type_name.clone().into(),
                primary_key: column.primary_key,
            },
        );
        TreeItem::new(column_id, column.name.clone())
    });
    TreeItem::new(id.clone(), table.table.label())
        .children(columns)
        .expanded(expanded.get(&id).copied().unwrap_or(false))
}

/// One shape per connection state: a dot when live, a ring while connecting,
/// the HELD seal when it failed.
pub fn state_shape(state: Option<&ConnectionState>, color: Rgba) -> AnyElement {
    match state {
        Some(ConnectionState::Live(_)) => div()
            .size_2()
            .flex_shrink_0()
            .rounded_full()
            .bg(color)
            .into_any_element(),
        Some(ConnectionState::Connecting) => div()
            .size_2()
            .flex_shrink_0()
            .rounded_full()
            .border_2()
            .border_color(color)
            .into_any_element(),
        Some(ConnectionState::Failed(_)) => div()
            .flex_shrink_0()
            .px_1()
            .bg(theme::STENCIL)
            .text_size(px(11.))
            .text_color(theme::SEAL)
            .child(t("HELD"))
            .into_any_element(),
        None => div().size_2().flex_shrink_0().into_any_element(),
    }
}

fn render_row(entry: &TreeEntry, node: Option<&Node>, selected: bool) -> ListItem {
    let item = entry.item();
    let chevron = if !entry.is_folder() {
        div().size_3().flex_shrink_0().into_any_element()
    } else if entry.is_expanded() {
        lucide("chevron-down")
            .size_3()
            .flex_shrink_0()
            .text_color(theme::STENCIL_MUTE)
            .into_any_element()
    } else {
        lucide("chevron-right")
            .size_3()
            .flex_shrink_0()
            .text_color(theme::STENCIL_MUTE)
            .into_any_element()
    };
    let label = div()
        .flex_1()
        .min_w_0()
        .truncate()
        .child(item.label.clone());
    let icon = |name: &'static str| {
        lucide(name)
            .size_3p5()
            .flex_shrink_0()
            .text_color(theme::STENCIL_MUTE)
    };

    let row = h_flex()
        .w_full()
        .min_w_0()
        .gap_1p5()
        .pl(px(12. * entry.depth() as f32))
        .child(chevron);
    let row = match node {
        Some(Node::Group { count, .. }) => row
            .child(icon("folder"))
            .child(label.text_color(theme::STENCIL))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme::STENCIL_FAINT)
                    .child(count.to_string()),
            ),
        Some(Node::Connection { id, state, line }) => row
            .child(div().w_1().h_3().flex_shrink_0().bg(*line))
            .child(icon("database"))
            .child(label.text_color(theme::STENCIL))
            .child(match state {
                // A seal dot; the reason waits in its tooltip.
                Some(ConnectionState::Failed(error)) => {
                    let error = error.clone();
                    div()
                        .id(SharedString::from(format!("failed:{id}")))
                        .size_2()
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(theme::SEAL)
                        .tooltip(move |window, cx| Tooltip::new(error.clone()).build(window, cx))
                        .into_any_element()
                }
                _ => state_shape(state.as_ref(), theme::STENCIL),
            }),
        Some(Node::Table { kind, .. }) => row
            .child(icon(match kind {
                ObjectKind::Table => "table-2",
                ObjectKind::View => "eye",
            }))
            .child(label),
        Some(Node::Column {
            type_name,
            primary_key,
        }) => row
            .child(if *primary_key {
                icon("key-round").into_any_element()
            } else {
                div().size_3p5().flex_shrink_0().into_any_element()
            })
            .child(label)
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(theme::STENCIL_FAINT)
                    .child(type_name.to_lowercase()),
            ),
        Some(Node::Note) => row.child(label.text_color(theme::STENCIL_FAINT)),
        None => row.child(label),
    };

    ListItem::new(item.id.clone())
        .h_8()
        .px_2()
        .py_0()
        .text_sm()
        .text_color(theme::STENCIL_DIM)
        .when(selected, |item| item.bg(theme::RIB))
        .child(row)
}

impl Render for Navigator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.store.read(cx).connections.is_empty();
        let nodes = self.nodes.clone();
        let menu_nodes = self.nodes.clone();
        let navigator = cx.weak_entity();
        let menu_navigator = cx.weak_entity();

        let body = if empty {
            div()
                .p_4()
                .text_sm()
                .text_color(theme::STENCIL_MUTE)
                .child(t("No connections yet"))
                .into_any_element()
        } else {
            tree(&self.tree, move |_, entry, selected, _, _| {
                let node = nodes.get(&entry.item().id);
                let row = render_row(entry, node, selected);
                let navigator = navigator.clone();
                match node.cloned() {
                    // A click opens a connection that is not open yet.
                    Some(Node::Connection { id, state, .. })
                        if !matches!(
                            state,
                            Some(ConnectionState::Live(_) | ConnectionState::Connecting)
                        ) =>
                    {
                        row.on_click(move |_, _, cx| {
                            let id = id.clone();
                            let _ = navigator.update(cx, |this, cx| this.connect(id, cx));
                        })
                    }
                    Some(Node::Table {
                        connection, table, ..
                    }) => row.on_click(move |event, _, cx| {
                        if event.click_count() == 2 {
                            let _ = navigator.update(cx, |_, cx| {
                                cx.emit(NavigatorEvent::OpenTable {
                                    connection: connection.clone(),
                                    table: table.clone(),
                                })
                            });
                        }
                    }),
                    _ => row,
                }
            })
            .context_menu(move |_, entry, menu, _, cx| {
                let Some(navigator) = menu_navigator.upgrade() else {
                    return menu;
                };
                if let Some(Node::Group { id, name, .. }) = menu_nodes.get(&entry.item().id) {
                    let store = navigator.read(cx).store.clone();
                    return group_menu(menu, navigator.downgrade(), store, id, name);
                }
                let Some(Node::Connection { id, state, .. }) = menu_nodes.get(&entry.item().id)
                else {
                    return menu;
                };
                let store = navigator.read(cx).store.clone();
                let Some(record) = store.read(cx).connection(id).cloned() else {
                    return menu;
                };
                let open = matches!(
                    state,
                    Some(ConnectionState::Live(_) | ConnectionState::Connecting)
                );
                let toggle = {
                    let (navigator, id) = (navigator.clone(), id.clone());
                    PopupMenuItem::new(if open { t("Disconnect") } else { t("Connect") }).on_click(
                        move |_, _, cx| {
                            navigator.update(cx, |this, cx| {
                                if open {
                                    this.disconnect(&id, cx);
                                } else {
                                    this.connect(id.clone(), cx);
                                }
                            });
                        },
                    )
                };
                let editor = {
                    let (navigator, id) = (navigator.clone(), id.clone());
                    PopupMenuItem::new(t("New SQL editor")).on_click(move |_, _, cx| {
                        navigator.update(cx, |_, cx| {
                            cx.emit(NavigatorEvent::NewEditor {
                                connection: id.clone(),
                            })
                        });
                    })
                };
                let edit = {
                    let id = id.clone();
                    PopupMenuItem::new(t("Edit")).on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| {
                            store.navigate(Route::ConnectionForm(Some(id.clone())), cx)
                        });
                    })
                };
                let delete = PopupMenuItem::new(t("Delete")).on_click(move |_, window, cx| {
                    let record = record.clone();
                    navigator.update(cx, |this, cx| this.delete(record, window, cx));
                });
                menu.item(toggle)
                    .item(editor)
                    .separator()
                    .item(edit)
                    .item(delete)
            })
            .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(theme::RAIL)
            .child(self.render_header(cx))
            .child(div().flex_1().min_h_0().py_1().child(body))
    }
}
