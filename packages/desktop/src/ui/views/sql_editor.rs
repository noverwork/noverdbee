//! A SQL editor tab: the editor over its results and messages.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::highlighter::Diagnostic;
use gpui_kit::component::input::{Editor, EditorState, InputEvent, RopeExt as _};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::{h_flex, v_flex, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::{Outcome, SessionManager, StatementResult};
use crate::store::history::HistoryEntry;
use crate::ui::bridge::backend;
use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::store::{store, Store};
use crate::ui::theme;
use crate::ui::views::result_grid::{self, GridState};
use crate::ui::views::sql_completion::SqlCompletion;

/// How long typing pauses before the SQL is checked.
const CHECK_DELAY: Duration = Duration::from_millis(500);

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
    /// The pending check of the SQL; replacing it cancels the old one.
    check: Option<Task<()>>,
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
                InputEvent::Change => {
                    // The error marker points into text that has now moved.
                    editor.update(cx, |editor, cx| {
                        if let Some(diagnostics) = editor.diagnostics_mut() {
                            if !diagnostics.is_empty() {
                                diagnostics.clear();
                                cx.notify();
                            }
                        }
                    });
                    this.schedule_check(cx);
                }
                _ => {}
            },
        )];
        let completion = Rc::new(SqlCompletion::default());
        editor.update(cx, |editor, _| {
            editor.lsp_mut().completion_provider = Some(completion.clone());
        });
        // Completion offers nothing but keywords until the tables load.
        let store = store(cx);
        if let Some(record) = store.read(cx).connection(&connection).cloned() {
            let app = store.read(cx).app();
            cx.spawn(async move |_, _| {
                let loaded = backend(async move {
                    let session = app.state::<SessionManager>().connect(&record).await?;
                    session.tables().await
                })
                .await;
                match loaded {
                    Ok(tables) => *completion.tables.borrow_mut() = tables,
                    Err(error) => tracing::warn!(%error, "cannot load tables for completion"),
                }
            })
            .detach();
        }
        Self {
            store,
            connection,
            editor,
            results: Vec::new(),
            messages: Vec::new(),
            active: 0,
            running: false,
            check: None,
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
                let sessions = app.state::<SessionManager>();
                let dropped = sessions.get(&record.id).is_some_and(|open| open.is_closed());
                let session = sessions.connect(&record).await?;
                let saved =
                    crate::store::history::history_add(record.id, sql.clone(), app.state()).await;
                if let Err(error) = saved {
                    tracing::warn!(error, "Failed to save query history");
                }
                Ok::<_, String>((dropped, session.run(sql).await))
            })
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.running = false;
                match ran {
                    Ok((dropped, results)) => {
                        this.show(results, base, window, cx);
                        if dropped {
                            this.messages.insert(
                                0,
                                Message {
                                    sql: SharedString::default(),
                                    text: t("The connection had closed and was opened again. Anything uncommitted on it was lost."),
                                    failed: false,
                                },
                            );
                        }
                    }
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

    /// Lists the SQL run on this connection; picking one puts it in the editor.
    fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let app = self.store.read(cx).app();
        let connection = self.connection.clone();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = backend(async move {
                crate::store::history::history_list(connection, app.state()).await
            })
            .await;
            let _ = this.update_in(cx, |_, window, cx| match loaded {
                Ok(entries) => show_history(entries, cx.weak_entity(), window, cx),
                Err(error) => window.push_notification(
                    gpui_kit::component::notification::Notification::error(error),
                    cx,
                ),
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

        self.mark(error.and_then(|(message, at)| Some((message, at?))), cx);
    }

    /// Has the database parse the SQL once typing pauses, and marks what it
    /// rejects.
    fn schedule_check(&mut self, cx: &mut Context<Self>) {
        let app = self.store.read(cx).app();
        let Some(session) = app.state::<SessionManager>().get(&self.connection) else {
            self.check = None;
            return;
        };
        let sql = self.editor.read(cx).value().to_string();
        if sql.trim().is_empty() {
            self.check = None;
            return;
        }
        self.check = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECK_DELAY).await;
            let checked = sql.clone();
            let found = backend(async move { session.check(checked).await }).await;
            let _ = this.update(cx, |this, cx| {
                // The text changed again, or a run marked its own error.
                if this.editor.read(cx).value().as_ref() != sql || this.running {
                    return;
                }
                this.mark(found, cx);
            });
        }));
    }

    /// Marks `error` at its byte offset in the editor, or clears the mark.
    fn mark(&self, error: Option<(String, usize)>, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let marker = error.map(|(message, at)| {
                let at = at.min(editor.text().len());
                let end = (at + 1).min(editor.text().len());
                let range =
                    editor.text().offset_to_position(at)..editor.text().offset_to_position(end);
                Diagnostic::new(range, message)
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
            .child(div().flex_1())
            .child(
                Button::new("history")
                    .ghost()
                    .small()
                    .icon(lucide("rotate-ccw-clock"))
                    .label(t("History"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_history(window, cx))),
            )
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

/// `11-18 14:02`, in local time.
fn ran_at(entry: &HistoryEntry) -> SharedString {
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(entry.ran_at, chrono::Utc)
        .with_timezone(&chrono::Local)
        .format("%m-%d %H:%M")
        .to_string()
        .into()
}

fn show_history(
    entries: Vec<HistoryEntry>,
    editor: WeakEntity<SqlEditor>,
    window: &mut Window,
    cx: &mut App,
) {
    let entries: std::rc::Rc<[HistoryEntry]> = entries.into();
    window.open_dialog(cx, move |dialog, _, _| {
        let body = if entries.is_empty() {
            div()
                .py_6()
                .text_sm()
                .text_color(theme::STENCIL_FAINT)
                .child(t("Nothing has run on this connection yet."))
                .into_any_element()
        } else {
            v_flex()
                .id("history")
                .max_h(px(480.))
                .overflow_y_scroll()
                .gap_1()
                .children(entries.iter().enumerate().map(|(index, entry)| {
                    let sql = entry.sql.clone();
                    let editor = editor.clone();
                    ListItem::new(("history-entry", index))
                        .py_2()
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .items_start()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(11.))
                                        .text_color(theme::STENCIL_MUTE)
                                        .child(ran_at(entry)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .line_clamp(3)
                                        .text_sm()
                                        .text_color(theme::STENCIL_DIM)
                                        .child(entry.sql.clone()),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            let _ =
                                editor.update(cx, |editor, cx| editor.set_text(&sql, window, cx));
                            window.close_dialog(cx);
                        })
                }))
                .into_any_element()
        };
        dialog.title(t("History")).w(px(720.)).child(body)
    })
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
