//! The data grid: a ruled ledger of rows with a row-number column, `NULL`
//! markers, and numbers on the right. Query results and table tabs share it.

use std::cmp::Ordering;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableState};
use gpui_kit::component::{Sizable as _, Size};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::{self, ResultSet, Value};
use crate::ui::i18n::t;
use crate::ui::theme;
use crate::ui::views::selection::{self, CopyCells, Selectable, Selection};

pub type GridState = TableState<Grid>;

/// Width of the row-number column.
const NUMBER_WIDTH: Pixels = px(56.);
/// Roughly one character of the 12px grid text.
const CHARACTER_WIDTH: f32 = 7.5;

pub struct Grid {
    result: ResultSet,
    /// Row indexes in display order; sorting reorders this, not the rows.
    order: Vec<usize>,
    selection: Selection,
}

impl Grid {
    pub fn new(result: ResultSet) -> Self {
        Self {
            order: (0..result.rows.len()).collect(),
            result,
            selection: Selection::default(),
        }
    }

    pub fn result(&self) -> &ResultSet {
        &self.result
    }

    fn value(&self, row_ix: usize, col_ix: usize) -> &Value {
        &self.result.rows[self.order[row_ix]][col_ix - 1]
    }

    fn row_text(&self, row_ix: usize) -> String {
        self.result.rows[self.order[row_ix]]
            .iter()
            .map(|value| value.as_deref().unwrap_or("NULL"))
            .collect::<Vec<_>>()
            .join("\t")
    }
}

/// Wide enough for the header and the widest of the first rows, within reason.
fn column_width(result: &ResultSet, index: usize) -> Pixels {
    let longest = result
        .rows
        .iter()
        .take(50)
        .map(|row| row[index].as_ref().map_or(4, |text| text.chars().count()))
        .chain([result.columns[index].name.chars().count()])
        .max()
        .unwrap_or(0);
    px((longest as f32 * CHARACTER_WIDTH + 24.).clamp(64., 320.))
}

/// Orders two cells of one column; NULL sorts first.
pub fn compare(left: &Value, right: &Value, numeric: bool) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(left), Some(right)) => {
            let numbers = numeric
                .then(|| left.parse::<f64>().ok().zip(right.parse::<f64>().ok()))
                .flatten();
            match numbers {
                Some((left, right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
                None => left.cmp(right),
            }
        }
    }
}

/// The text of one cell: the value, or the `NULL` marker.
pub fn cell(value: &Value) -> Div {
    match value {
        Some(text) => div().truncate().child(SharedString::from(text.clone())),
        None => div()
            .text_size(px(11.))
            .text_color(theme::STENCIL_FAINT)
            .child("NULL"),
    }
}

impl Selectable for Grid {
    fn selection(&self) -> &Selection {
        &self.selection
    }

    fn selection_mut(&mut self) -> &mut Selection {
        &mut self.selection
    }

    fn value(&self, row: usize, col: usize) -> Value {
        self.value(row, col).clone()
    }
}

impl TableDelegate for Grid {
    fn columns_count(&self, _: &App) -> usize {
        self.result.columns.len() + 1
    }

    fn rows_count(&self, _: &App) -> usize {
        self.order.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        if col_ix == 0 {
            return Column::new("#", "#")
                .width(NUMBER_WIDTH)
                .fixed_left()
                .text_right()
                .p_0()
                .resizable(false)
                .movable(false)
                .selectable(false);
        }
        let column = &self.result.columns[col_ix - 1];
        let grid_column = Column::new(format!("column-{col_ix}"), column.name.clone())
            .width(column_width(&self.result, col_ix - 1))
            .p_0()
            .movable(false)
            .sortable();
        if column.numeric {
            grid_column.text_right()
        } else {
            grid_column
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        cx: &mut Context<GridState>,
    ) {
        self.order = (0..self.result.rows.len()).collect();
        if col_ix > 0 && sort != ColumnSort::Default {
            let numeric = self.result.columns[col_ix - 1].numeric;
            let rows = &self.result.rows;
            self.order.sort_by(|left, right| {
                let ordering =
                    compare(&rows[*left][col_ix - 1], &rows[*right][col_ix - 1], numeric);
                if sort == ColumnSort::Descending {
                    ordering.reverse()
                } else {
                    ordering
                }
            });
        }
        cx.notify();
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        cx: &mut Context<GridState>,
    ) -> PopupMenu {
        let copy = |text: String| {
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            }
        };
        let picked = self
            .selection
            .picked()
            .filter(|picked| picked.has_row(row_ix));
        let copied = picked.and_then(|_| selection::copied(self, cx));
        menu.when_some(copied, |menu, text| {
            menu.item(PopupMenuItem::new(t("Copy")).on_click(copy(text)))
        })
        .item(PopupMenuItem::new(t("Copy row")).on_click(copy(self.row_text(row_ix))))
    }

    /// The cells draw their own padding, so the header does too.
    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<GridState>,
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
        cx: &mut Context<GridState>,
    ) -> impl IntoElement {
        let picked = self.selection.contains(row_ix, col_ix);
        let content = if col_ix == 0 {
            div()
                .text_color(theme::STENCIL_FAINT)
                .child((row_ix + 1).to_string())
        } else {
            cell(self.value(row_ix, col_ix))
        };
        selection::cell_area(content, row_ix, col_ix, picked, cx)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        if col_ix == 0 {
            return (row_ix + 1).to_string();
        }
        self.value(row_ix, col_ix).clone().unwrap_or_default()
    }
}

pub fn new_grid(result: ResultSet, window: &mut Window, cx: &mut App) -> Entity<GridState> {
    let grid = cx.new(|cx| {
        TableState::new(Grid::new(result), window, cx)
            .cell_selectable(true)
            .row_selectable(false)
            .row_header(false)
            .col_movable(false)
    });
    selection::track(&grid, cx);
    grid
}

/// The grid element for `state`.
pub fn grid(state: &Entity<GridState>) -> impl IntoElement {
    let copy = state.clone();
    div()
        .size_full()
        .text_xs()
        .on_action(move |_: &CopyCells, _, cx| {
            if let Some(text) = selection::copied(copy.read(cx).delegate(), cx) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        })
        .child(
            DataTable::new(state)
                .with_size(Size::Size(px(24.)))
                .bordered(false)
                .header_row_height(px(24.)),
        )
}

/// `120 rows`, or `10000+ rows` for a result that was cut short.
pub fn row_count(result: &ResultSet) -> SharedString {
    let count = result.rows.len();
    if result.truncated {
        format!("{}+", db::ROW_LIMIT).into()
    } else {
        count.to_string().into()
    }
}

#[cfg(test)]
mod tests {
    use super::compare;
    use std::cmp::Ordering;

    #[test]
    fn numbers_sort_by_value_and_null_sorts_first() {
        let value = |text: &str| Some(text.to_string());
        assert_eq!(compare(&value("9"), &value("10"), true), Ordering::Less);
        assert_eq!(compare(&value("9"), &value("10"), false), Ordering::Greater);
        assert_eq!(compare(&None, &value("0"), true), Ordering::Less);
        assert_eq!(compare(&value("b"), &value("a"), true), Ordering::Greater);
    }
}
