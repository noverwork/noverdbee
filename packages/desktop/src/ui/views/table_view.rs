//! A table tab: browse and edit its rows, or read how it is defined. Edits
//! are staged in the grid and written only when the user saves them.

use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::table::{
    Column, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Sizable as _, Size, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::sql::{self, Direction};
use crate::db::{
    self, ObjectKind, Outcome, ResultSet, SessionManager, StatementResult, TableInfo, TableRef,
    Value,
};
use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::store::{store, Store};
use crate::ui::theme;
use crate::ui::views::common;
use crate::ui::views::result_grid::{self, GridState};
use crate::ui::views::selection::{self, CopyCells, PasteCells, Selectable, Selection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowState {
    Saved,
    New,
    Deleted,
}

#[derive(Debug, Clone)]
struct Row {
    /// The values as loaded; empty for a new row.
    values: Vec<Value>,
    /// Staged values by column.
    edits: HashMap<usize, Value>,
    state: RowState,
}

impl Row {
    fn saved(values: Vec<Value>) -> Self {
        Self {
            values,
            edits: HashMap::new(),
            state: RowState::Saved,
        }
    }

    fn value(&self, column: usize) -> &Value {
        self.edits
            .get(&column)
            .or_else(|| self.values.get(column))
            .unwrap_or(&None)
    }

    fn is_dirty(&self) -> bool {
        self.state != RowState::Saved || !self.edits.is_empty()
    }
}

/// The statements that write the staged changes, one per changed row.
fn statements(table: &TableRef, columns: &[String], key: &[usize], rows: &[Row]) -> Vec<String> {
    let pairs = |row: &Row, indexes: &mut dyn Iterator<Item = usize>, staged: bool| {
        indexes
            .map(|index| {
                let value = if staged {
                    row.value(index)
                } else {
                    &row.values[index]
                };
                (columns[index].clone(), value.clone())
            })
            .collect::<Vec<_>>()
    };
    rows.iter()
        .filter(|row| row.is_dirty())
        .map(|row| {
            let mut changed: Vec<usize> = row.edits.keys().copied().collect();
            changed.sort_unstable();
            match row.state {
                RowState::New => sql::insert(table, &pairs(row, &mut changed.into_iter(), true)),
                RowState::Deleted => {
                    sql::delete(table, &pairs(row, &mut key.iter().copied(), false))
                }
                RowState::Saved => sql::update(
                    table,
                    &pairs(row, &mut key.iter().copied(), false),
                    &pairs(row, &mut changed.into_iter(), true),
                ),
            }
        })
        .collect()
}

/// Why a save must be rolled back, if it must. Every staged statement has to
/// change exactly the one row it was written for.
fn save_failure(results: &[StatementResult], expected: usize) -> Option<String> {
    for (index, result) in results.iter().enumerate() {
        match &result.outcome {
            Outcome::Error { message, .. } => return Some(message.clone()),
            // The first result is BEGIN and the last is COMMIT.
            Outcome::Affected(count) if (1..=expected).contains(&index) && *count != 1 => {
                return Some(format!(
                    "statement {index} changed {count} rows instead of 1"
                ))
            }
            _ => {}
        }
    }
    (results.len() != expected + 2).then(|| "the save did not finish".to_string())
}

struct Editing {
    row: usize,
    column: usize,
    input: Entity<InputState>,
}

pub struct EditGrid {
    columns: Vec<db::Column>,
    /// Indexes of the primary-key columns; empty makes the grid read-only.
    key: Vec<usize>,
    rows: Vec<Row>,
    more: bool,
    order: Option<(usize, Direction)>,
    editing: Option<Editing>,
    selection: Selection,
    view: WeakEntity<TableView>,
}

impl EditGrid {
    fn pending(&self) -> usize {
        self.rows.iter().filter(|row| row.is_dirty()).count()
    }

    fn editable(&self) -> bool {
        !self.key.is_empty()
    }

    /// Stages `value` for a cell, or drops the edit when it equals what was loaded.
    fn stage(&mut self, row: usize, column: usize, value: Value) {
        let Some(row) = self.rows.get_mut(row) else {
            return;
        };
        if row.state != RowState::New && row.values.get(column) == Some(&value) {
            row.edits.remove(&column);
        } else {
            row.edits.insert(column, value);
        }
    }

    /// Stages a saved row for deletion or takes that back; drops a new row.
    fn toggle_delete(&mut self, row: usize) {
        match self.rows.get(row).map(|row| row.state) {
            Some(RowState::New) => {
                self.rows.remove(row);
            }
            Some(RowState::Saved) => self.rows[row].state = RowState::Deleted,
            Some(RowState::Deleted) => self.rows[row].state = RowState::Saved,
            None => {}
        }
    }

    /// Stages `rows` for deletion, dropping new ones, or restores them.
    fn set_deleted(&mut self, rows: &[usize], delete: bool) {
        // From the bottom, so dropping a new row leaves the others in place.
        for &row in rows.iter().rev() {
            match self.rows.get(row).map(|row| row.state) {
                Some(RowState::New) if delete => {
                    self.rows.remove(row);
                }
                Some(RowState::Saved) if delete => self.rows[row].state = RowState::Deleted,
                Some(RowState::Deleted) if !delete => self.rows[row].state = RowState::Saved,
                _ => {}
            }
        }
    }

    /// Stages pasted `lines` over the picked cells; rows staged for deletion keep theirs.
    fn paste(&mut self, lines: &[Vec<Value>]) {
        let Some(picked) = self.selection.picked() else {
            return;
        };
        let targets = selection::spread(&picked, lines, self.rows.len(), self.columns.len());
        for (row, col, value) in targets {
            if self.rows[row].state != RowState::Deleted {
                self.stage(row, col - 1, value);
            }
        }
    }

    fn row_text(&self, row: usize) -> String {
        (0..self.columns.len())
            .map(|column| self.rows[row].value(column).as_deref().unwrap_or("NULL"))
            .collect::<Vec<_>>()
            .join("\t")
    }
}

impl Selectable for EditGrid {
    fn selection(&self) -> &Selection {
        &self.selection
    }

    fn selection_mut(&mut self) -> &mut Selection {
        &mut self.selection
    }

    fn value(&self, row: usize, col: usize) -> Value {
        self.rows[row].value(col - 1).clone()
    }
}

impl TableDelegate for EditGrid {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len() + 1
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        if col_ix == 0 {
            return Column::new("#", "#")
                .width(px(56.))
                .fixed_left()
                .text_right()
                .p_0()
                .resizable(false)
                .movable(false)
                .selectable(false);
        }
        let index = col_ix - 1;
        let column = &self.columns[index];
        let longest = self
            .rows
            .iter()
            .take(50)
            .map(|row| {
                row.value(index)
                    .as_ref()
                    .map_or(4, |text| text.chars().count())
            })
            .chain([column.name.chars().count()])
            .max()
            .unwrap_or(0);
        let grid_column = Column::new(format!("column-{col_ix}"), column.name.clone())
            .width(px((longest as f32 * 7.5 + 24.).clamp(64., 320.)))
            .p_0()
            .movable(false)
            .sortable();
        let grid_column = match self.order {
            Some((ordered, Direction::Ascending)) if ordered == index => grid_column.ascending(),
            Some((ordered, Direction::Descending)) if ordered == index => grid_column.descending(),
            _ => grid_column,
        };
        if column.numeric {
            grid_column.text_right()
        } else {
            grid_column
        }
    }

    /// Sorting asks the database again, so it is refused while edits are staged.
    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        if col_ix == 0 || self.pending() > 0 {
            return;
        }
        let order = match sort {
            ColumnSort::Ascending => Some((col_ix - 1, Direction::Ascending)),
            ColumnSort::Descending => Some((col_ix - 1, Direction::Descending)),
            ColumnSort::Default => None,
        };
        let view = self.view.clone();
        // The view reads this grid while it reloads, so let this update finish
        // first. `window.defer`, not `cx.defer_in`: that one runs inside an
        // update of this grid again.
        window.defer(cx, move |window, cx| {
            let _ = view.update(cx, |view, cx| {
                view.order = order;
                view.load(Load::Replace, window, cx);
            });
        });
    }

    fn has_more(&self, _: &App) -> bool {
        self.more
    }

    fn load_more(&mut self, window: &mut Window, cx: &mut Context<TableState<Self>>) {
        let view = self.view.clone();
        // See `perform_sort`.
        window.defer(cx, move |window, cx| {
            let _ = view.update(cx, |view, cx| view.load(Load::Append, window, cx));
        });
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let grid = cx.weak_entity();
        let copy = |text: String| {
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            }
        };
        let picked = self
            .selection
            .picked()
            .filter(|picked| picked.has_row(row_ix));
        let copied = picked.as_ref().and_then(|_| selection::copied(self, cx));
        let menu = menu
            .when_some(copied, |menu, text| {
                menu.item(PopupMenuItem::new(t("Copy")).on_click(copy(text)))
            })
            .item(PopupMenuItem::new(t("Copy row")).on_click(copy(self.row_text(row_ix))));
        if !self.editable() {
            return menu;
        }
        let rows = picked
            .as_ref()
            .map_or_else(|| vec![row_ix], |picked| picked.rows(self.rows.len()));
        let cells: Vec<(usize, usize)> = picked
            .map(|picked| {
                let cols = picked.cols(self.columns.len());
                rows.iter()
                    .flat_map(|&row| cols.clone().map(move |col| (row, col - 1)))
                    .collect()
            })
            .unwrap_or_default();
        let delete = self.rows[row_ix].state != RowState::Deleted;
        let change = |grid: WeakEntity<TableState<Self>>, apply: Box<dyn Fn(&mut Self)>| {
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = grid.update(cx, |grid, cx| {
                    apply(grid.delegate_mut());
                    cx.notify();
                });
            }
        };
        let label = match (delete, rows.len() > 1) {
            (true, false) => t("Delete row"),
            (true, true) => t("Delete rows"),
            (false, false) => t("Restore row"),
            (false, true) => t("Restore rows"),
        };
        menu.separator()
            .when(!cells.is_empty() && delete, |menu| {
                menu.item(PopupMenuItem::new(t("Set NULL")).on_click(change(
                    grid.clone(),
                    Box::new(move |this| {
                        for &(row, column) in &cells {
                            if this
                                .rows
                                .get(row)
                                .is_some_and(|row| row.state != RowState::Deleted)
                            {
                                this.stage(row, column, None);
                            }
                        }
                    }),
                )))
            })
            .item(PopupMenuItem::new(label).on_click(change(
                grid,
                Box::new(move |this| this.set_deleted(&rows, delete)),
            )))
    }

    /// The cells draw their own padding, so the header does too.
    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .w_full()
            .px_2()
            .child(self.column(col_ix, cx).name.clone())
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let picked = self.selection.contains(row_ix, col_ix);
        let row = &self.rows[row_ix];
        if col_ix == 0 {
            let number = div()
                .text_color(match row.state {
                    RowState::Saved => theme::STENCIL_FAINT,
                    RowState::New => theme::AMBER_TEXT,
                    RowState::Deleted => theme::RED_TEXT,
                })
                .child((row_ix + 1).to_string());
            return selection::cell_area(number, row_ix, col_ix, picked, cx).into_any_element();
        }
        let column = col_ix - 1;
        if let Some(editing) = &self.editing {
            if editing.row == row_ix && editing.column == column {
                return div()
                    .size_full()
                    .px_1()
                    .child(Input::new(&editing.input).xsmall().appearance(false))
                    .into_any_element();
            }
        }
        let cell = result_grid::cell(row.value(column)).w_full();
        let cell = match row.state {
            RowState::Deleted => cell.text_color(theme::STENCIL_FAINT).line_through(),
            _ if row.edits.contains_key(&column) => cell
                .px_1()
                .border_1()
                .border_color(theme::AMBER)
                .text_color(theme::AMBER_TEXT),
            _ => cell,
        };
        selection::cell_area(cell, row_ix, col_ix, picked, cx).into_any_element()
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        if col_ix == 0 {
            return (row_ix + 1).to_string();
        }
        self.rows[row_ix]
            .value(col_ix - 1)
            .clone()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Data,
    Structure,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Load {
    Replace,
    Append,
}

/// One part of a table's definition: a grid of facts, or its DDL text.
enum Section {
    Grid(SharedString, Entity<GridState>),
    Text(SharedString, SharedString),
}

pub struct TableView {
    store: Entity<Store>,
    connection: String,
    info: TableInfo,
    mode: Mode,
    filter: Entity<InputState>,
    grid: Option<Entity<TableState<EditGrid>>>,
    order: Option<(usize, Direction)>,
    loading: bool,
    saving: bool,
    error: Option<SharedString>,
    structure: Vec<Section>,
    section: usize,
    _subscriptions: Vec<Subscription>,
    _edit: Option<Subscription>,
}

impl TableView {
    pub fn new(
        connection: String,
        info: TableInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("condition, e.g. id > 100")));
        let _subscriptions = vec![cx.subscribe_in(
            &filter,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.reload(window, cx);
                }
            },
        )];
        let mut view = Self {
            store: store(cx),
            connection,
            info,
            mode: Mode::Data,
            filter,
            grid: None,
            order: None,
            loading: false,
            saving: false,
            error: None,
            structure: Vec::new(),
            section: 0,
            _subscriptions,
            _edit: None,
        };
        view.load(Load::Replace, window, cx);
        view
    }

    pub fn table(&self) -> &TableRef {
        &self.info.table
    }

    /// How many rows have staged changes.
    pub fn pending(&self, cx: &App) -> usize {
        self.grid
            .as_ref()
            .map_or(0, |grid| grid.read(cx).delegate().pending())
    }

    pub fn is_busy(&self) -> bool {
        self.loading || self.saving
    }

    /// `200 rows`, for the status line.
    pub fn summary(&self, cx: &App) -> Option<SharedString> {
        let grid = self.grid.as_ref()?.read(cx).delegate();
        let count = grid.rows.len().to_string();
        Some(if grid.more {
            tf("{count} rows, more available", &[("count", &count)])
        } else {
            tf("{count} rows", &[("count", &count)])
        })
    }

    /// Runs `sql` on this tab's connection.
    fn run(
        &self,
        sql: String,
        cx: &App,
    ) -> impl std::future::Future<Output = Result<Vec<StatementResult>, String>> {
        let record = self.store.read(cx).connection(&self.connection).cloned();
        let app = self.store.read(cx).app();
        backend(async move {
            let record = record.ok_or("the connection was deleted")?;
            let session = app.state::<SessionManager>().connect(&record).await?;
            Ok(session.run(sql).await)
        })
    }

    /// Reloads from the first row, asking first when edits would be lost.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending(cx) == 0 {
            self.load(Load::Replace, window, cx);
            return;
        }
        let view = cx.weak_entity();
        common::confirm_delete(
            t("Discard changes?"),
            t("Reloading drops the changes that have not been saved."),
            None,
            t("Discard"),
            window,
            cx,
            move |window, cx| {
                let _ = view.update(cx, |view, cx| view.load(Load::Replace, window, cx));
            },
        );
    }

    fn load(&mut self, load: Load, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let offset = match (&self.grid, load) {
            (Some(grid), Load::Append) => grid
                .read(cx)
                .delegate()
                .rows
                .iter()
                .filter(|row| row.state != RowState::New)
                .count(),
            _ => 0,
        };
        let order = self.order.and_then(|(index, direction)| {
            Some((self.info.columns.get(index)?.name.clone(), direction))
        });
        let query = sql::page_query(
            &self.info.table,
            &self.filter.read(cx).value(),
            order
                .as_ref()
                .map(|(name, direction)| (name.as_str(), *direction)),
            db::PAGE_SIZE,
            offset,
        );
        let ran = self.run(query, cx);
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let ran = ran.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading = false;
                let page =
                    ran.and_then(|mut results| match results.pop().map(|last| last.outcome) {
                        Some(Outcome::Rows(page)) => Ok(page),
                        Some(Outcome::Error { message, .. }) => Err(message),
                        _ => Err("the table returned no rows".to_string()),
                    });
                match page {
                    Ok(page) => this.show(page, load, window, cx),
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn show(&mut self, page: ResultSet, load: Load, window: &mut Window, cx: &mut Context<Self>) {
        let more = page.rows.len() == db::PAGE_SIZE;
        let rows = page.rows.into_iter().map(Row::saved);
        if let (Some(grid), Load::Append) = (&self.grid, load) {
            grid.update(cx, |grid, cx| {
                let delegate = grid.delegate_mut();
                delegate.rows.extend(rows);
                delegate.more = more;
                cx.notify();
            });
            return;
        }
        // Views and tables without a primary key cannot be edited: there is
        // no way to say which row a change belongs to.
        let key = if self.info.kind == ObjectKind::Table {
            page.columns
                .iter()
                .enumerate()
                .filter(|(_, column)| {
                    self.info
                        .columns
                        .iter()
                        .any(|info| info.primary_key && info.name == column.name)
                })
                .map(|(index, _)| index)
                .collect()
        } else {
            Vec::new()
        };
        let delegate = EditGrid {
            columns: page.columns,
            key,
            rows: rows.collect(),
            more,
            order: self.order,
            editing: None,
            selection: Selection::default(),
            view: cx.weak_entity(),
        };
        let grid = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .cell_selectable(true)
                .row_selectable(false)
                .row_header(false)
                .col_movable(false)
        });
        self._subscriptions.truncate(1);
        self._subscriptions
            .push(cx.observe(&grid, |_, _, cx| cx.notify()));
        self._subscriptions.push(cx.subscribe_in(
            &grid,
            window,
            |this, _, event: &TableEvent, window, cx| {
                if let TableEvent::DoubleClickedCell(row, column) = event {
                    if *column > 0 {
                        this.edit(*row, *column - 1, window, cx);
                    }
                }
            },
        ));
        selection::track(&grid, cx);
        self.grid = Some(grid);
    }

    /// Puts an input in the cell.
    fn edit(&mut self, row: usize, column: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(grid) = self.grid.clone() else {
            return;
        };
        let current = {
            let delegate = grid.read(cx).delegate();
            match delegate.rows.get(row) {
                Some(found) if delegate.editable() && found.state != RowState::Deleted => {
                    found.value(column).clone().unwrap_or_default()
                }
                _ => return,
            }
        };
        self.commit(cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        self._edit = Some(cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit(cx);
            }
        }));
        input.update(cx, |input, cx| input.focus(window, cx));
        grid.update(cx, |grid, cx| {
            grid.delegate_mut().editing = Some(Editing { row, column, input });
            cx.notify();
        });
    }

    /// Stages what was typed in the cell being edited.
    fn commit(&mut self, cx: &mut Context<Self>) {
        let Some(grid) = self.grid.clone() else {
            return;
        };
        grid.update(cx, |grid, cx| {
            if let Some(editing) = grid.delegate_mut().editing.take() {
                let text = editing.input.read(cx).value().to_string();
                grid.delegate_mut()
                    .stage(editing.row, editing.column, Some(text));
                cx.notify();
            }
        });
    }

    /// Stages the clipboard over the picked cells.
    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let Some(grid) = self.grid.clone() else {
            return;
        };
        self.commit(cx);
        let lines = selection::parse(&text);
        grid.update(cx, |grid, cx| {
            if grid.delegate().editable() {
                grid.delegate_mut().paste(&lines);
                cx.notify();
            }
        });
    }

    /// Stages `value` for a cell, as typing it into the cell would.
    pub fn stage(&mut self, row: usize, column: usize, value: Value, cx: &mut Context<Self>) {
        if let Some(grid) = &self.grid {
            grid.update(cx, |grid, cx| {
                grid.delegate_mut().stage(row, column, value);
                cx.notify();
            });
        }
    }

    /// Stages a row for deletion, or takes that back.
    pub fn toggle_delete(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(grid) = &self.grid {
            grid.update(cx, |grid, cx| {
                grid.delegate_mut().toggle_delete(row);
                cx.notify();
            });
        }
    }

    pub fn add_row(&mut self, cx: &mut Context<Self>) {
        let Some(grid) = self.grid.clone() else {
            return;
        };
        grid.update(cx, |grid, cx| {
            let rows = &mut grid.delegate_mut().rows;
            rows.push(Row {
                values: Vec::new(),
                edits: HashMap::new(),
                state: RowState::New,
            });
            let last = rows.len() - 1;
            grid.scroll_to_row(last, cx);
        });
    }

    fn statements(&self, cx: &App) -> Vec<String> {
        let Some(grid) = &self.grid else {
            return Vec::new();
        };
        let delegate = grid.read(cx).delegate();
        let columns: Vec<String> = delegate
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        statements(&self.info.table, &columns, &delegate.key, &delegate.rows)
    }

    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        let text: SharedString = self.statements(cx).join(";\n\n").into();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(t("Pending changes")).w(px(720.)).child(
                div()
                    .id("review-sql")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .p_3()
                    .rounded(theme::RADIUS)
                    .bg(theme::INK)
                    .text_xs()
                    .text_color(theme::STENCIL_DIM)
                    .child(text.clone()),
            )
        });
    }

    /// Writes every staged change in one transaction, or none of them.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(cx);
        let statements = self.statements(cx);
        if statements.is_empty() || self.saving {
            return;
        }
        let expected = statements.len();
        let ran = self.run(format!("BEGIN;\n{};\nCOMMIT;", statements.join(";\n")), cx);
        let undo = self.run("ROLLBACK".to_string(), cx);
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let failure = match ran.await {
                Ok(results) => save_failure(&results, expected),
                Err(error) => Some(error),
            };
            if failure.is_some() {
                let _ = undo.await;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match failure {
                    Some(error) => this.error = Some(error.into()),
                    None => this.load(Load::Replace, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn show_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::Structure;
        cx.notify();
        if !self.structure.is_empty() {
            return;
        }
        let Some(record) = self.store.read(cx).connection(&self.connection).cloned() else {
            return;
        };
        let app = self.store.read(cx).app();
        let table = self.info.table.clone();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = backend(async move {
                let session = app.state::<SessionManager>().connect(&record).await?;
                let queries = session.structure_queries(&table);
                let sql: Vec<String> = queries.iter().map(|(_, sql)| sql.clone()).collect();
                let results = session.run(sql.join(";\n")).await;
                Ok::<_, String>((queries, results))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match loaded {
                    Ok((queries, results)) => {
                        for ((title, _), result) in queries.into_iter().zip(results) {
                            let title = t(title);
                            match result.outcome {
                                Outcome::Rows(rows) if title == t("Definition") => {
                                    let text = rows
                                        .rows
                                        .into_iter()
                                        .flatten()
                                        .flatten()
                                        .collect::<Vec<_>>()
                                        .join("\n\n");
                                    this.structure.push(Section::Text(title, text.into()));
                                }
                                Outcome::Rows(rows) => this.structure.push(Section::Grid(
                                    title,
                                    result_grid::new_grid(rows, window, cx),
                                )),
                                Outcome::Error { message, .. } => {
                                    this.structure.push(Section::Text(title, message.into()))
                                }
                                Outcome::Affected(_) => {}
                            }
                        }
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let mode = |id: &'static str, label: SharedString, mode: Mode| {
            let button = Button::new(id).small().label(label);
            if self.mode == mode {
                button.primary()
            } else {
                button.ghost()
            }
        };
        let editable = self
            .grid
            .as_ref()
            .is_some_and(|grid| grid.read(cx).delegate().editable());
        h_flex()
            .h_8()
            .flex_shrink_0()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(theme::RIB)
            .bg(theme::RAIL)
            .child(
                mode("mode-data", t("Data"), Mode::Data).on_click(cx.listener(|this, _, _, cx| {
                    this.mode = Mode::Data;
                    cx.notify();
                })),
            )
            .child(
                mode("mode-structure", t("Structure"), Mode::Structure)
                    .on_click(cx.listener(|this, _, window, cx| this.show_structure(window, cx))),
            )
            .when(self.mode == Mode::Data, |toolbar| {
                toolbar
                    .child(
                        div()
                            .pl_2()
                            .text_size(px(11.))
                            .text_color(theme::STENCIL_MUTE)
                            .child("WHERE"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.filter).xsmall().cleanable(true)),
                    )
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .small()
                            .icon(lucide("refresh-cw"))
                            .tooltip(t("Refresh"))
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    )
                    .when(editable, |toolbar| {
                        toolbar.child(
                            Button::new("add-row")
                                .ghost()
                                .small()
                                .icon(lucide("plus"))
                                .label(t("Row"))
                                .on_click(cx.listener(|this, _, _, cx| this.add_row(cx))),
                        )
                    })
            })
    }

    fn render_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let pending = self.pending(cx);
        let editable = self
            .grid
            .as_ref()
            .is_none_or(|grid| grid.read(cx).delegate().editable());
        let left = if pending > 0 {
            h_flex()
                .gap_2()
                .child(div().text_color(theme::AMBER_TEXT).child(tf(
                    "{count} pending changes",
                    &[("count", &pending.to_string())],
                )))
                .child(
                    Button::new("review")
                        .ghost()
                        .xsmall()
                        .label(t("Review SQL"))
                        .on_click(cx.listener(|this, _, window, cx| this.review(window, cx))),
                )
                .child(
                    Button::new("discard")
                        .ghost()
                        .xsmall()
                        .label(t("Discard"))
                        .disabled(self.saving)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.load(Load::Replace, window, cx)),
                        ),
                )
                .child(
                    Button::new("save")
                        .warning()
                        .xsmall()
                        .label(t("Save"))
                        .loading(self.saving)
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                )
        } else if !editable {
            h_flex().child(if self.info.kind == ObjectKind::View {
                t("Read-only: this is a view")
            } else {
                t("Read-only: no primary key")
            })
        } else {
            h_flex()
        };
        h_flex()
            .h_7()
            .flex_shrink_0()
            .justify_between()
            .px_3()
            .bg(theme::RAIL)
            .border_t_1()
            .border_color(theme::RIB)
            .text_size(px(11.))
            .text_color(theme::STENCIL_MUTE)
            .child(left)
            .children(self.summary(cx))
    }

    fn render_data(&self, cx: &Context<Self>) -> impl IntoElement {
        let body = match &self.grid {
            Some(grid) => div()
                .size_full()
                .text_xs()
                .on_action({
                    let grid = grid.clone();
                    move |_: &CopyCells, _, cx| {
                        if let Some(text) = selection::copied(grid.read(cx).delegate(), cx) {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    }
                })
                .on_action(cx.listener(|this, _: &PasteCells, _, cx| this.paste(cx)))
                .child(
                    DataTable::new(grid)
                        .with_size(Size::Size(px(24.)))
                        .bordered(false)
                        .header_row_height(px(24.)),
                )
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(theme::STENCIL_FAINT)
                .child(if self.loading {
                    t("Loading...")
                } else {
                    t("No rows")
                })
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .children(
                self.error
                    .clone()
                    .map(|error| div().p_2().child(common::error_banner(error))),
            )
            .child(div().flex_1().min_h_0().child(body))
            .child(self.render_footer(cx))
    }

    fn render_structure(&self, cx: &Context<Self>) -> impl IntoElement {
        let tabs = h_flex()
            .h_6()
            .flex_shrink_0()
            .bg(theme::RAIL)
            .border_b_1()
            .border_color(theme::RIB)
            .children(self.structure.iter().enumerate().map(|(index, section)| {
                let title = match section {
                    Section::Grid(title, _) | Section::Text(title, _) => title.clone(),
                };
                let active = index == self.section;
                div()
                    .id(("section", index))
                    .h_full()
                    .flex()
                    .items_center()
                    .px_3()
                    .border_b_1()
                    .text_size(px(11.))
                    .cursor_pointer()
                    .border_color(if active {
                        Hsla::from(theme::STENCIL)
                    } else {
                        gpui_kit::transparent_black()
                    })
                    .text_color(if active {
                        theme::STENCIL
                    } else {
                        theme::STENCIL_MUTE
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = index;
                        cx.notify();
                    }))
                    .child(SharedString::from(title.to_uppercase()))
            }));
        let body = match self.structure.get(self.section) {
            Some(Section::Grid(_, grid)) => result_grid::grid(grid).into_any_element(),
            Some(Section::Text(_, text)) => div()
                .id("definition")
                .size_full()
                .overflow_y_scroll()
                .p_4()
                .bg(theme::INK)
                .text_sm()
                .text_color(theme::STENCIL_DIM)
                .child(text.clone())
                .into_any_element(),
            None => div().into_any_element(),
        };
        v_flex()
            .size_full()
            .child(tabs)
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl Render for TableView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(theme::PORT)
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(match self.mode {
                Mode::Data => self.render_data(cx).into_any_element(),
                Mode::Structure => self.render_structure(cx).into_any_element(),
            }))
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would pull in GPUI's own `test` attribute.
    use super::{save_failure, statements, Row, RowState};
    use crate::db::{Outcome, StatementResult, TableRef};
    use std::collections::HashMap;
    use std::time::Duration;

    fn value(text: &str) -> Option<String> {
        Some(text.to_string())
    }

    fn row(values: &[&str], edits: &[(usize, Option<&str>)], state: RowState) -> Row {
        Row {
            values: values.iter().map(|text| value(text)).collect(),
            edits: edits
                .iter()
                .map(|(column, text)| (*column, text.map(str::to_string)))
                .collect::<HashMap<_, _>>(),
            state,
        }
    }

    #[test]
    fn staged_changes_become_one_statement_per_row_keyed_on_the_original_key() {
        let table = TableRef {
            schema: None,
            name: "users".to_string(),
        };
        let columns = ["id".to_string(), "name".to_string(), "note".to_string()];
        let rows = [
            row(&["1", "ann", "x"], &[], RowState::Saved),
            // Changing the key itself still finds the row by its old key.
            row(
                &["2", "bo", "y"],
                &[(0, Some("20")), (2, None)],
                RowState::Saved,
            ),
            row(
                &["3", "cy", "z"],
                &[(1, Some("ignored"))],
                RowState::Deleted,
            ),
            row(&[], &[(1, Some("dee"))], RowState::New),
            row(&[], &[], RowState::New),
        ];
        assert_eq!(
            statements(&table, &columns, &[0], &rows),
            [
                r#"UPDATE "users" SET "id" = '20', "note" = NULL WHERE "id" = '2'"#,
                r#"DELETE FROM "users" WHERE "id" = '3'"#,
                r#"INSERT INTO "users" ("name") VALUES ('dee')"#,
                r#"INSERT INTO "users" DEFAULT VALUES"#,
            ]
        );
    }

    #[test]
    fn a_save_fails_unless_every_statement_changed_one_row() {
        let result = |sql: &str, outcome: Outcome| StatementResult {
            sql: sql.to_string(),
            outcome,
            elapsed: Duration::ZERO,
        };
        let begin = || result("BEGIN", Outcome::Affected(0));
        let commit = || result("COMMIT", Outcome::Affected(0));
        let update = |count: u64| result("UPDATE t SET x = 1", Outcome::Affected(count));

        assert_eq!(save_failure(&[begin(), update(1), commit()], 1), None);
        assert!(save_failure(&[begin(), update(0), commit()], 1).is_some());
        assert!(save_failure(&[begin(), update(3), commit()], 1).is_some());
        let error = Outcome::Error {
            message: "constraint failed".to_string(),
            position: None,
        };
        assert_eq!(
            save_failure(&[begin(), result("UPDATE", error)], 1).as_deref(),
            Some("constraint failed")
        );
        assert!(save_failure(&[begin(), update(1)], 2).is_some());
    }
}
