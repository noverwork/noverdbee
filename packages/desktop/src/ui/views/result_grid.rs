//! The data grid: a ruled ledger of rows with a row-number column, `NULL`
//! markers, and numbers on the right. Query results and table tabs share it.

use std::cmp::Ordering;

use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableState};
use gpui_kit::component::{Sizable as _, Size, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::{self, ResultSet, Value};
use crate::ui::i18n::t;
use crate::ui::theme;
use crate::ui::views::selection::{self, CopyCells, Picked, Selectable, Selection};

pub type GridState = TableState<Grid>;

/// Width of the row-number column.
const NUMBER_WIDTH: Pixels = px(56.);
/// Roughly one character of the 14px grid text.
const CHARACTER_WIDTH: f32 = 8.5;

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

/// The most characters a cell shows: the whole text is shaped each time its
/// row scrolls into view, and no column is this wide.
const CELL_CHARS: usize = 500;

/// The text of one cell: the value on one line, or the `NULL` marker.
pub fn cell(value: &Value) -> Div {
    match value {
        Some(text) => {
            let shown = text
                .char_indices()
                .nth(CELL_CHARS)
                .map_or(text.as_str(), |(end, _)| &text[..end]);
            // A line break would grow the cell past its row.
            div()
                .truncate()
                .child(SharedString::from(shown.replace(['\r', '\n'], " ")))
        }
        None => div()
            .text_size(px(11.))
            .text_color(theme::STENCIL_FAINT)
            .child("NULL"),
    }
}

/// The picked cell laid out one member per line, when it is the only one
/// picked, sits on `row_ix`, and holds a JSON object or array.
pub fn picked_json<D: Selectable>(grid: &D, row_ix: usize) -> Option<String> {
    let Some(Picked::Block { rows, cols }) = grid.selection().picked() else {
        return None;
    };
    if rows != (row_ix..row_ix + 1) || cols.len() != 1 {
        return None;
    }
    pretty_json(&grid.value(row_ix, cols.start)?)
}

/// `text` indented, when it is a JSON object or array. Reindents the text as
/// it is rather than parsing it, so key order and big numbers stay exact.
fn pretty_json(text: &str) -> Option<String> {
    if !text.trim_start().starts_with(['{', '['])
        || serde_json::from_str::<serde::de::IgnoredAny>(text).is_err()
    {
        return None;
    }
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    let mut out = String::with_capacity(text.len() * 2);
    let (mut depth, mut in_string, mut escaped) = (0, false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                // `{}` and `[]` stay on one line.
                if let Some(close) = chars.next_if(|&c| c == '}' || c == ']') {
                    out.push(close);
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth -= 1;
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            _ if c.is_whitespace() => {}
            _ => out.push(c),
        }
    }
    Some(out)
}

/// Adds "View JSON" to a cell's menu when `json` is there.
pub fn view_json_item(menu: PopupMenu, json: Option<String>) -> PopupMenu {
    menu.when_some(json, |menu, json| {
        menu.item(
            PopupMenuItem::new(t("View JSON"))
                .on_click(move |_, window, cx| show_json(&json, window, cx)),
        )
    })
}

fn show_json(json: &str, window: &mut Window, cx: &mut App) {
    let editor = cx.new(|cx| {
        EditorState::new(window, cx)
            .language("json")
            .default_value(json.to_string())
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("JSON")
            .w(px(720.))
            .child(Editor::new(&editor).readonly(true).h(px(480.)))
    });
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
        let menu = menu
            .when_some(copied, |menu, text| {
                menu.item(PopupMenuItem::new(t("Copy")).on_click(copy(text)))
            })
            .item(PopupMenuItem::new(t("Copy row")).on_click(copy(self.row_text(row_ix))));
        view_json_item(menu, picked_json(self, row_ix))
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
        .text_sm()
        .on_action(move |_: &CopyCells, _, cx| {
            if let Some(text) = selection::copied(copy.read(cx).delegate(), cx) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        })
        .child(
            DataTable::new(state)
                .with_size(Size::Size(px(32.)))
                .bordered(false)
                .header_row_height(px(32.)),
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

/// `result` as CSV (RFC 4180): a header line, then one line per row. A NULL
/// is an empty field; quotes go around a value that needs them, and around
/// an empty or `NULL` text so the import reads it back as text.
pub fn to_csv(result: &ResultSet) -> String {
    let field = |text: &str| {
        if text.is_empty() || text == "NULL" || text.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_string()
        }
    };
    let header = result.columns.iter().map(|column| field(&column.name));
    let mut lines = vec![header.collect::<Vec<_>>().join(",")];
    lines.extend(result.rows.iter().map(|row| {
        row.iter()
            .map(|value| value.as_deref().map(field).unwrap_or_default())
            .collect::<Vec<_>>()
            .join(",")
    }));
    lines.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::{compare, pretty_json, to_csv};
    use crate::db::{Column, ResultSet};

    #[test]
    fn pretty_json_keeps_order_numbers_and_strings() {
        let text = r#"{"z":12345678901234567890,"a":[],"s":"a,b:{\"c\"}","o":{ "x" : [1,{}] }}"#;
        assert_eq!(
            pretty_json(text).as_deref(),
            Some(concat!(
                "{\n",
                "  \"z\": 12345678901234567890,\n",
                "  \"a\": [],\n",
                "  \"s\": \"a,b:{\\\"c\\\"}\",\n",
                "  \"o\": {\n",
                "    \"x\": [\n",
                "      1,\n",
                "      {}\n",
                "    ]\n",
                "  }\n",
                "}"
            ))
        );
        assert_eq!(pretty_json("42"), None);
        assert_eq!(pretty_json("{not json"), None);
    }

    #[test]
    fn csv_quotes_what_needs_it_and_leaves_null_empty() {
        let column = |name: &str| Column {
            name: name.to_string(),
            type_name: String::new(),
            numeric: false,
        };
        let result = ResultSet {
            columns: vec![column("id"), column("note, short")],
            rows: vec![
                vec![Some("1".to_string()), Some("say \"hi\"\nbye".to_string())],
                vec![Some("2".to_string()), None],
            ],
            truncated: false,
        };
        assert_eq!(
            to_csv(&result),
            "id,\"note, short\"\r\n1,\"say \"\"hi\"\"\nbye\"\r\n2,\r\n"
        );
        // The import reads back exactly what was exported.
        let awkward = ResultSet {
            rows: vec![
                vec![Some(String::new()), Some("NULL".to_string())],
                vec![None, None],
            ],
            ..result.clone()
        };
        let lines = crate::ui::views::selection::parse_csv(&to_csv(&awkward));
        assert_eq!(lines[1..], awkward.rows);
    }
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
