//! A SQL editor tab: the editor over its results and messages.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::highlighter::Diagnostic;
use gpui_kit::component::input::{Editor, EditorState, InputEvent, RopeExt as _};
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::{h_flex, v_flex, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::{Outcome, SessionManager, StatementResult};
use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::store::{store, Store};
use crate::ui::theme;
use crate::ui::views::result_grid::{self, GridState};

struct ResultTab {
    grid: Entity<GridState>,
    /// `120 rows · 34 ms`
    summary: SharedString,
}

struct Message {
    sql: SharedString,
    text: SharedString,
    failed: bool,
}

pub struct SqlEditor {
    store: Entity<Store>,
    connection: String,
    editor: Entity<EditorState>,
    results: Vec<ResultTab>,
    messages: Vec<Message>,
    /// The result tab shown; `results.len()` is the Messages tab.
    active: usize,
    running: bool,
    _subscriptions: Vec<Subscription>,
}

/// The first line of a statement, for the message list.
fn first_line(sql: &str) -> SharedString {
    sql.trim()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
        .into()
}

impl SqlEditor {
    pub fn new(connection: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .placeholder(t("Write SQL. Cmd+Enter runs the selection, or everything."))
        });
        let _subscriptions = vec![cx.subscribe_in(
            &editor,
            window,
            |this, editor, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: true, ..
                } => this.run(window, cx),
                // The error marker points into text that has now moved.
                InputEvent::Change => editor.update(cx, |editor, cx| {
                    if let Some(diagnostics) = editor.diagnostics_mut() {
                        if !diagnostics.is_empty() {
                            diagnostics.clear();
                            cx.notify();
                        }
                    }
                }),
                _ => {}
            },
        )];
        Self {
            store: store(cx),
            connection,
            editor,
            results: Vec::new(),
            messages: Vec::new(),
            active: 0,
            running: false,
            _subscriptions,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// The last result shown, for the status line.
    pub fn summary(&self) -> Option<SharedString> {
        self.results
            .get(self.active)
            .map(|result| result.summary.clone())
    }

    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.set_value(text.to_string(), window, cx)
        });
    }

    /// Runs the selection, or the whole editor when nothing is selected.
    // ponytail: no "statement under the cursor"; that needs a SQL splitter
    // that understands strings, comments and dollar quotes.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let (sql, base) = {
            let editor = self.editor.read(cx);
            let text = editor.value();
            let range = editor.selected_range();
            match text.get(range.clone()) {
                Some(selected) if !range.is_empty() => (selected.to_string(), range.start),
                _ => (text.to_string(), 0),
            }
        };
        if sql.trim().is_empty() {
            return;
        }
        let Some(record) = self.store.read(cx).connection(&self.connection).cloned() else {
            return;
        };
        let app = self.store.read(cx).app();
        self.running = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let ran = backend(async move {
                let session = app.state::<SessionManager>().connect(&record).await?;
                Ok::<_, String>(session.run(sql).await)
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.running = false;
                match ran {
                    Ok(results) => this.show(results, base, window, cx),
                    Err(error) => {
                        this.results.clear();
                        this.messages = vec![Message {
                            sql: SharedString::default(),
                            text: error.into(),
                            failed: true,
                        }];
                        this.active = 0;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let app = self.store.read(cx).app();
        if let Some(session) = app.state::<SessionManager>().get(&self.connection) {
            session.cancel();
        }
    }

    /// Replaces what is shown with `results`. `base` is where the SQL that was
    /// run starts in the editor, for placing the error marker.
    fn show(
        &mut self,
        results: Vec<StatementResult>,
        base: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.results.clear();
        self.messages.clear();
        let mut error = None;
        for result in results {
            let elapsed = result.elapsed.as_millis().to_string();
            let (text, failed) = match result.outcome {
                Outcome::Rows(rows) => {
                    let count = result_grid::row_count(&rows);
                    let summary = tf(
                        "{count} rows · {elapsed} ms",
                        &[("count", &count), ("elapsed", &elapsed)],
                    );
                    self.results.push(ResultTab {
                        grid: result_grid::new_grid(rows, window, cx),
                        summary: summary.clone(),
                    });
                    (summary, false)
                }
                Outcome::Affected(count) => (
                    tf(
                        "{count} rows affected · {elapsed} ms",
                        &[("count", &count.to_string()), ("elapsed", &elapsed)],
                    ),
                    false,
                ),
                Outcome::Error { message, position } => {
                    error = Some((message.clone(), position.map(|at| base + at)));
                    (message.into(), true)
                }
            };
            self.messages.push(Message {
                sql: first_line(&result.sql),
                text,
                failed,
            });
        }
        // Rows first; an error, or a run with nothing to show, opens Messages.
        self.active = if error.is_some() {
            self.results.len()
        } else {
            0
        };

        self.editor.update(cx, |editor, cx| {
            let marker = error.and_then(|(message, at)| {
                let at = at?.min(editor.text().len());
                let end = (at + 1).min(editor.text().len());
                let range =
                    editor.text().offset_to_position(at)..editor.text().offset_to_position(end);
                Some(Diagnostic::new(range, message))
            });
            if let Some(diagnostics) = editor.diagnostics_mut() {
                diagnostics.clear();
                if let Some(marker) = marker {
                    diagnostics.push(marker);
                }
            }
            cx.notify();
        });
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let action = if self.running {
            Button::new("cancel")
                .outline()
                .small()
                .icon(lucide("square"))
                .label(t("Cancel"))
                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
        } else {
            Button::new("run")
                .primary()
                .small()
                .icon(lucide("play"))
                .label(t("Run"))
                .tooltip(t("Cmd+Enter"))
                .on_click(cx.listener(|this, _, window, cx| this.run(window, cx)))
        };
        h_flex()
            .h_8()
            .flex_shrink_0()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(theme::RIB)
            .bg(theme::RAIL)
            .child(action)
    }

    fn render_result_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        let tab = |index: usize, label: SharedString| {
            let active = index == self.active;
            div()
                .id(("result-tab", index))
                .h_full()
                .flex()
                .items_center()
                .px_3()
                .border_b_1()
                .text_size(px(11.))
                .cursor_pointer()
                .map(|tab| {
                    if active {
                        tab.border_color(theme::STENCIL).text_color(theme::STENCIL)
                    } else {
                        tab.border_color(gpui_kit::transparent_black())
                            .text_color(theme::STENCIL_MUTE)
                            .hover(|style| style.text_color(theme::STENCIL))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active = index;
                    cx.notify();
                }))
                .child(label)
        };
        h_flex()
            .h_6()
            .flex_shrink_0()
            .bg(theme::RAIL)
            .border_b_1()
            .border_color(theme::RIB)
            .children((0..self.results.len()).map(|index| {
                tab(
                    index,
                    tf("RESULT {number}", &[("number", &(index + 1).to_string())]),
                )
            }))
            .child(tab(self.results.len(), t("MESSAGES")))
    }

    fn render_messages(&self) -> impl IntoElement {
        v_flex()
            .id("messages")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .text_sm()
            .children(self.messages.iter().map(|message| {
                v_flex()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .rounded(theme::RADIUS)
                    .border_1()
                    .border_color(if message.failed {
                        theme::SEAL.opacity(0.6)
                    } else {
                        theme::hairline()
                    })
                    .when(!message.sql.is_empty(), |entry| {
                        entry.child(
                            div()
                                .truncate()
                                .text_color(theme::STENCIL_MUTE)
                                .child(message.sql.clone()),
                        )
                    })
                    .child(
                        div()
                            .text_color(if message.failed {
                                theme::RED_TEXT
                            } else {
                                theme::STENCIL_DIM
                            })
                            .child(message.text.clone()),
                    )
            }))
    }

    fn render_results(&self, cx: &Context<Self>) -> impl IntoElement {
        let idle = self.results.is_empty() && self.messages.is_empty();
        let body = if self.running || idle {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme::STENCIL_FAINT)
                .child(if self.running {
                    t("Running...")
                } else {
                    t("Results appear here.")
                })
                .into_any_element()
        } else if let Some(result) = self.results.get(self.active) {
            result_grid::grid(&result.grid).into_any_element()
        } else {
            self.render_messages().into_any_element()
        };
        v_flex()
            .size_full()
            .bg(theme::PORT)
            .when(!idle, |pane| pane.child(self.render_result_tabs(cx)))
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl Render for SqlEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(self.render_toolbar(cx)).child(
            div().flex_1().min_h_0().child(
                v_resizable("sql-editor")
                    .child(
                        resizable_panel().child(
                            div().size_full().bg(theme::INK).child(
                                Editor::new(&self.editor)
                                    .h_full()
                                    .appearance(false)
                                    .bordered(false),
                            ),
                        ),
                    )
                    .child(resizable_panel().child(self.render_results(cx))),
            ),
        )
    }
}
