//! Picking a block of cells or several whole rows in a data grid, and moving
//! them through the clipboard as tab-separated text.
//!
//! Column indexes are the table's: 0 is the row-number column. The table
//! itself still selects one cell, the focus; a drag or a Shift-click spans
//! from the anchor to it. Clicks on the row-number column pick whole rows.

use std::collections::BTreeSet;
use std::ops::Range;

use gpui_kit::component::table::{TableDelegate, TableEvent, TableState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::Value;
use crate::ui::theme;

actions!(noverdbee, [CopyCells, PasteCells]);

/// The data grid's key context, set by `DataTable`.
const CONTEXT: &str = "DataTable";

pub fn bind_keys(cx: &mut App) {
    // Cmd on macOS; Ctrl elsewhere.
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(format!("{modifier}-c").leak(), CopyCells, Some(CONTEXT)),
        KeyBinding::new(format!("{modifier}-v").leak(), PasteCells, Some(CONTEXT)),
    ]);
}

/// A grid whose cells can be picked several at a time.
pub trait Selectable: TableDelegate + Sized {
    fn selection(&self) -> &Selection;
    fn selection_mut(&mut self) -> &mut Selection;
    /// The value of a data cell; `col` is a table column, so at least 1.
    fn value(&self, row: usize, col: usize) -> Value;
}

#[derive(Debug, Default)]
pub struct Selection {
    anchor: Option<(usize, usize)>,
    focus: Option<(usize, usize)>,
    /// The picked rows while the anchor is in the row-number column.
    rows: BTreeSet<usize>,
}

/// The picked cells.
#[derive(Debug, PartialEq, Eq)]
pub enum Picked<'a> {
    Block {
        rows: Range<usize>,
        cols: Range<usize>,
    },
    Rows(&'a BTreeSet<usize>),
}

fn span(from: usize, to: usize) -> Range<usize> {
    from.min(to)..from.max(to) + 1
}

impl Selection {
    /// A press on `cell`. Shift spans from the anchor; `toggle` (Cmd, or Ctrl
    /// elsewhere) adds a row to the picked rows or takes it out.
    pub fn press(&mut self, cell: (usize, usize), shift: bool, toggle: bool) {
        let (row, col) = cell;
        let in_rows = self.anchor.is_some_and(|anchor| anchor.1 == 0);
        match self.anchor {
            Some(anchor) if shift && in_rows == (col == 0) => {
                if col == 0 {
                    self.rows = span(anchor.0, row).collect();
                }
            }
            Some(_) if toggle && in_rows && col == 0 => {
                if !self.rows.remove(&row) {
                    self.rows.insert(row);
                }
                self.anchor = Some(cell);
            }
            _ => {
                self.anchor = Some(cell);
                self.rows = if col == 0 {
                    BTreeSet::from([row])
                } else {
                    BTreeSet::new()
                };
            }
        }
        self.focus = Some(cell);
    }

    /// The pointer was dragged onto `cell`; returns the new focus when it moved.
    pub fn drag(&mut self, cell: (usize, usize)) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        let focus = if anchor.1 == 0 {
            (cell.0, 0)
        } else {
            (cell.0, cell.1.max(1))
        };
        if self.focus == Some(focus) {
            return None;
        }
        if anchor.1 == 0 {
            self.rows = span(anchor.0, focus.0).collect();
        }
        self.focus = Some(focus);
        Some(focus)
    }

    /// The table selected `cell` on its own, from the keyboard: start over there.
    pub fn sync(&mut self, cell: Option<(usize, usize)>) {
        if self.focus != cell {
            *self = Self::default();
            if let Some(cell) = cell {
                self.press(cell, false, false);
            }
        }
    }

    pub fn picked(&self) -> Option<Picked<'_>> {
        let (anchor, focus) = (self.anchor?, self.focus?);
        Some(if anchor.1 == 0 {
            Picked::Rows(&self.rows)
        } else {
            Picked::Block {
                rows: span(anchor.0, focus.0),
                cols: span(anchor.1, focus.1),
            }
        })
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        self.picked()
            .is_some_and(|picked| picked.contains(row, col))
    }
}

impl Picked<'_> {
    pub fn contains(&self, row: usize, col: usize) -> bool {
        match self {
            Picked::Block { rows, cols } => rows.contains(&row) && cols.contains(&col),
            Picked::Rows(rows) => rows.contains(&row),
        }
    }

    pub fn has_row(&self, row: usize) -> bool {
        match self {
            Picked::Block { rows, .. } => rows.contains(&row),
            Picked::Rows(rows) => rows.contains(&row),
        }
    }

    /// The picked rows that still exist, top to bottom.
    pub fn rows(&self, rows_count: usize) -> Vec<usize> {
        match self {
            Picked::Block { rows, .. } => rows.start.min(rows_count)..rows.end.min(rows_count),
            Picked::Rows(rows) => return rows.range(..rows_count).copied().collect(),
        }
        .collect()
    }

    /// The picked table columns; whole rows take every data column.
    pub fn cols(&self, columns: usize) -> Range<usize> {
        match self {
            Picked::Block { cols, .. } => cols.start..cols.end.min(columns + 1),
            Picked::Rows(_) => 1..columns + 1,
        }
    }
}

/// Covers a whole cell around `content`: shows whether the cell is picked, and
/// picks it on a press or a drag.
pub fn cell_area<D: Selectable>(
    content: impl IntoElement,
    row: usize,
    col: usize,
    picked: bool,
    cx: &mut Context<TableState<D>>,
) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .px_2()
        .when(picked, |cell| cell.bg(theme::picked()))
        .child(div().w_full().min_w_0().child(content))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |table, event: &MouseDownEvent, _, cx| {
                let toggle = event.modifiers.secondary(cx);
                table.delegate_mut().selection_mut().press(
                    (row, col),
                    event.modifiers.shift,
                    toggle,
                );
                table.set_selected_cell(row, col, cx);
            }),
        )
        .on_mouse_move(cx.listener(move |table, event: &MouseMoveEvent, _, cx| {
            if event.pressed_button != Some(MouseButton::Left) {
                return;
            }
            if let Some((row, col)) = table.delegate_mut().selection_mut().drag((row, col)) {
                table.set_selected_cell(row, col, cx);
            }
        }))
}

/// Keeps the picked cells in step with what the table selects from the
/// keyboard. The subscription lives as long as the grid.
pub fn track<D: Selectable + 'static>(grid: &Entity<TableState<D>>, cx: &mut App) {
    cx.subscribe(grid, |grid, event: &TableEvent, cx| {
        let cell = match event {
            TableEvent::SelectCell(row, col) => Some((*row, *col)),
            TableEvent::ClearSelection => None,
            _ => return,
        };
        grid.update(cx, |grid, cx| {
            grid.delegate_mut().selection_mut().sync(cell);
            cx.notify();
        });
    })
    .detach();
}

/// The picked cells as tab-separated text.
pub fn copied<D: Selectable>(grid: &D, cx: &App) -> Option<String> {
    let picked = grid.selection().picked()?;
    let cols = picked.cols(grid.columns_count(cx) - 1);
    let rows = picked.rows(grid.rows_count(cx));
    let lines: Vec<Vec<Value>> = rows
        .into_iter()
        .map(|row| cols.clone().map(|col| grid.value(row, col)).collect())
        .collect();
    Some(to_text(&lines))
}

/// Tab-separated lines; `NULL` for a null, quotes around a value that needs them.
pub fn to_text(lines: &[Vec<Value>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|value| match value {
                    None => "NULL".to_string(),
                    Some(text) if text.contains(['\t', '\n', '\r', '"']) || text == "NULL" => {
                        format!("\"{}\"", text.replace('"', "\"\""))
                    }
                    Some(text) => text.clone(),
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pasted text as lines of values: tab-separated when it has a tab, CSV
/// otherwise. Quotes follow CSV; an unquoted `NULL` is a null.
pub fn parse(text: &str) -> Vec<Vec<Value>> {
    let delimiter = if text.contains('\t') { '\t' } else { ',' };
    split(text, delimiter, false)
}

/// A CSV file as lines of values. An unquoted empty field or `NULL` is a
/// null, which reads back what the CSV export writes.
pub fn parse_csv(text: &str) -> Vec<Vec<Value>> {
    split(text, ',', true)
}

fn split(text: &str, delimiter: char, empty_is_null: bool) -> Vec<Vec<Value>> {
    let value = |field: &mut String, quoted: &mut bool| {
        let field = std::mem::take(field);
        let null = field == "NULL" || (empty_is_null && field.is_empty());
        (std::mem::take(quoted) || !null).then_some(field)
    };
    let mut lines = Vec::new();
    let mut line = Vec::new();
    let mut field = String::new();
    let (mut in_quotes, mut quoted) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => (in_quotes, quoted) = (true, true),
            '\r' => {}
            '\n' => {
                line.push(value(&mut field, &mut quoted));
                lines.push(std::mem::take(&mut line));
            }
            c if c == delimiter => line.push(value(&mut field, &mut quoted)),
            _ => field.push(c),
        }
    }
    if !field.is_empty() || quoted || !line.is_empty() {
        line.push(value(&mut field, &mut quoted));
        lines.push(line);
    }
    lines
}

/// Where pasted `lines` go, as `(row, table column, value)`. One picked cell
/// takes the whole block from there; a larger pick is filled, the block
/// repeating across it. Nothing lands past the last row or column.
pub fn spread(
    picked: &Picked,
    lines: &[Vec<Value>],
    rows_count: usize,
    columns: usize,
) -> Vec<(usize, usize, Value)> {
    let width = lines.iter().map(Vec::len).max().unwrap_or(0);
    if width == 0 {
        return Vec::new();
    }
    let (rows, cols) = match picked {
        Picked::Block { rows, cols } if rows.len() == 1 && cols.len() == 1 => (
            (rows.start..(rows.start + lines.len()).min(rows_count)).collect(),
            cols.start..(cols.start + width).min(columns + 1),
        ),
        _ => (picked.rows(rows_count), picked.cols(columns)),
    };
    let mut targets = Vec::new();
    for (i, row) in rows.into_iter().enumerate() {
        let line = &lines[i % lines.len()];
        for (j, col) in cols.clone().enumerate() {
            if let Some(value) = line.get(j % width) {
                targets.push((row, col, value.clone()));
            }
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's own `test` attribute.
    use super::{parse, spread, to_text, Picked, Selection, Value};
    use std::collections::BTreeSet;

    fn value(text: &str) -> Value {
        Some(text.to_string())
    }

    #[test]
    fn csv_and_tsv_parse_with_quotes_and_nulls() {
        assert_eq!(
            parse("a,\"b,c\",NULL\r\n\"x\"\"y\",\"NULL\",\n"),
            vec![
                vec![value("a"), value("b,c"), None],
                vec![value("x\"y"), value("NULL"), value("")],
            ]
        );
        assert_eq!(
            parse("1\t2,3\n\"multi\nline\"\t4"),
            vec![
                vec![value("1"), value("2,3")],
                vec![value("multi\nline"), value("4")]
            ]
        );
        let lines = vec![vec![None, value("a\tb")], vec![value("NULL"), value("")]];
        assert_eq!(parse(&to_text(&lines)), lines);
    }

    #[test]
    fn a_pick_spans_drags_and_toggles() {
        let mut selection = Selection::default();
        selection.press((2, 3), false, false);
        selection.drag((4, 1));
        assert_eq!(
            selection.picked(),
            Some(Picked::Block {
                rows: 2..5,
                cols: 1..4
            })
        );
        // The table moved its cell from the keyboard: back to one cell.
        selection.sync(Some((5, 1)));
        assert_eq!(
            selection.picked(),
            Some(Picked::Block {
                rows: 5..6,
                cols: 1..2
            })
        );
        selection.press((1, 0), false, false);
        selection.press((3, 0), true, false);
        selection.press((7, 0), false, true);
        selection.press((2, 0), false, true);
        assert_eq!(
            selection.picked(),
            Some(Picked::Rows(&BTreeSet::from([1, 3, 7])))
        );
    }

    #[test]
    fn a_paste_fills_the_pick_or_spreads_from_one_cell() {
        let lines = parse("a,b\nc,d");
        let block = Picked::Block {
            rows: 0..3,
            cols: 1..5,
        };
        let filled = spread(&block, &lines, 10, 10);
        assert_eq!(filled.len(), 12);
        assert_eq!(filled[0], (0, 1, value("a")));
        assert_eq!(filled[3], (0, 4, value("b")));
        assert_eq!(filled[11], (2, 4, value("b")));

        let one = Picked::Block {
            rows: 9..10,
            cols: 2..3,
        };
        assert_eq!(
            spread(&one, &lines, 10, 2),
            vec![(9, 2, value("a"))],
            "clipped at the last row and column"
        );
    }
}
