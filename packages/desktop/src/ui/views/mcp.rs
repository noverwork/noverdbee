//! The MCP page: the port, how to connect an AI client, the tools it gets,
//! and what it has done. Laid out like noverterm's MCP page.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use crate::mcp::{McpActivity, McpStatus};
use crate::ui::bridge::backend;
use crate::ui::i18n::t;
use crate::ui::icons::lucide;
use crate::ui::store::{store, Store};
use crate::ui::theme;
use crate::ui::views::common;

const MASK: &str = "••••••••";
/// How often the page rereads the activity log while it is open.
const POLL: Duration = Duration::from_secs(2);

/// `(label, config file)`; no file means the snippet is a shell command.
const CLIENTS: [(&str, Option<&str>); 4] = [
    ("Claude Code", None),
    ("Cursor", Some("~/.cursor/mcp.json")),
    ("VS Code", Some(".vscode/mcp.json")),
    ("Claude Desktop", Some("claude_desktop_config.json")),
];

const TOOLS: [(&str, &str); 7] = [
    (
        "list_connections",
        "Lists the connections shared with MCP and the tools each allows.",
    ),
    ("list_tables", "Lists the tables and views of a connection."),
    ("describe_table", "Lists a table's columns."),
    ("select", "Runs one read-only query."),
    ("insert", "Inserts rows into a table."),
    ("update", "Updates the rows matching a condition."),
    ("delete", "Deletes the rows matching a condition."),
];

/// What to paste into the client at `CLIENTS[client]` to connect it.
fn client_snippet(client: usize, url: &str, token: &str) -> String {
    let auth = format!("Bearer {token}");
    let config = match client {
        0 => {
            return format!(
                "claude mcp add --transport http noverdbee {url} --header \"Authorization: {auth}\""
            )
        }
        1 => {
            json!({ "mcpServers": { "noverdbee": { "url": url, "headers": { "Authorization": auth } } } })
        }
        2 => json!({
            "servers": { "noverdbee": { "type": "http", "url": url, "headers": { "Authorization": auth } } }
        }),
        _ => json!({
            "mcpServers": {
                "noverdbee": {
                    "command": "npx",
                    "args": ["-y", "mcp-remote", url, "--header", "Authorization:${AUTH_HEADER}"],
                    "env": { "AUTH_HEADER": auth },
                }
            }
        }),
    };
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

pub struct McpView {
    store: Entity<Store>,
    status: McpStatus,
    activity: Vec<McpActivity>,
    port: Entity<InputState>,
    saving: bool,
    error: Option<SharedString>,
    /// Index into `CLIENTS`.
    client: usize,
    reveal: bool,
    /// Which copy button was just used.
    copied: Option<&'static str>,
    confirming_regenerate: bool,
    _poll: Task<()>,
    _copied: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl McpView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let app = store.read(cx).app();
        let status = crate::mcp::status(&app);
        let port = cx.new(|cx| InputState::new(window, cx).default_value(status.port.to_string()));
        let _subscriptions = vec![cx.subscribe(&port, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        let _poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                break;
            }
        });
        Self {
            activity: crate::mcp::activity(&app),
            error: status.error.clone().map(SharedString::from),
            status,
            store,
            port,
            saving: false,
            client: 0,
            reveal: false,
            copied: None,
            confirming_regenerate: false,
            _poll,
            _copied: None,
            _subscriptions,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let app = self.store.read(cx).app();
        self.status = crate::mcp::status(&app);
        self.activity = crate::mcp::activity(&app);
        cx.notify();
    }

    /// Runs a backend call, then shows the new status or its error.
    fn apply<F>(&mut self, call: F, window: &mut Window, cx: &mut Context<Self>)
    where
        F: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = backend(call).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                this.reload(cx);
                let port = this.status.port.to_string();
                this.port
                    .update(cx, |input, cx| input.set_value(port, window, cx));
                this.error = result.err().map(SharedString::from);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_port(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.port.read(cx).value().trim().parse::<u16>() {
            Ok(port) if port > 0 => {
                let app = self.store.read(cx).app();
                self.apply(crate::mcp::set_port(app, port), window, cx);
            }
            _ => {
                self.error = Some(t("Port must be between 1 and 65535"));
                cx.notify();
            }
        }
    }

    fn copy(&mut self, key: &'static str, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(key);
        self._copied = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn copy_button(&self, key: &'static str, text: String, cx: &Context<Self>) -> Button {
        let copied = self.copied == Some(key);
        Button::new(key)
            .ghost()
            .xsmall()
            .icon(if copied {
                lucide("check").text_color(theme::GREEN_TEXT)
            } else {
                lucide("copy")
            })
            .label(t("Copy"))
            .on_click(cx.listener(move |this, _, _, cx| this.copy(key, text.clone(), cx)))
    }

    fn render_token(&self, token: &str, cx: &Context<Self>) -> Div {
        let row = h_flex()
            .gap_2()
            .min_w_0()
            .child(div().truncate().text_xs().child(if self.reveal {
                SharedString::from(token.to_string())
            } else {
                MASK.into()
            }))
            .child(
                Button::new("reveal-token")
                    .ghost()
                    .xsmall()
                    .icon(lucide(if self.reveal { "eye-off" } else { "eye" }))
                    .tooltip(if self.reveal { t("Hide") } else { t("Show") })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reveal = !this.reveal;
                        cx.notify();
                    })),
            )
            .child(self.copy_button("copy-token", token.to_string(), cx));
        if !self.confirming_regenerate {
            return row.child(
                Button::new("regenerate")
                    .ghost()
                    .xsmall()
                    .icon(lucide("refresh-cw"))
                    .label(t("Regenerate"))
                    .disabled(self.saving)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirming_regenerate = true;
                        cx.notify();
                    })),
            );
        }
        row.child(div().text_xs().text_color(theme::STENCIL_MUTE).child(t(
            "Clients using the current token will stop working. Regenerate it?",
        )))
        .child(
            Button::new("cancel-regenerate")
                .ghost()
                .xsmall()
                .label(t("Cancel"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.confirming_regenerate = false;
                    cx.notify();
                })),
        )
        .child(
            Button::new("confirm-regenerate")
                .ghost()
                .xsmall()
                .text_color(theme::RED_TEXT)
                .label(t("Regenerate"))
                .disabled(self.saving)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.confirming_regenerate = false;
                    let app = this.store.read(cx).app();
                    this.apply(crate::mcp::regenerate_token(app), window, cx);
                })),
        )
    }

    fn render_client(&self, url: &str, token: &str, cx: &Context<Self>) -> Div {
        let tabs = CLIENTS.iter().enumerate().map(|(index, (label, _))| {
            let selected = self.client == index;
            div()
                .id(*label)
                .h_7()
                .px_3()
                .flex()
                .items_center()
                .rounded(theme::RADIUS)
                .border_1()
                .text_xs()
                .cursor_pointer()
                .map(|tab| {
                    if selected {
                        tab.border_color(theme::STENCIL).text_color(theme::STENCIL)
                    } else {
                        tab.border_color(theme::hairline())
                            .text_color(theme::STENCIL_MUTE)
                            .hover(|style| style.text_color(theme::STENCIL))
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.client = index;
                    cx.notify();
                }))
                .child(*label)
        });
        let file: SharedString = match CLIENTS[self.client].1 {
            Some(file) => file.into(),
            None => t("Terminal"),
        };
        let shown = client_snippet(self.client, url, if self.reveal { token } else { MASK });
        v_flex()
            .gap_3()
            .child(common::section_title(t("Connect a client")))
            .child(h_flex().flex_wrap().gap_1().children(tabs))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(div().text_xs().text_color(theme::STENCIL_MUTE).child(file))
                            .child(self.copy_button(
                                "copy-snippet",
                                client_snippet(self.client, url, token),
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded(theme::RADIUS)
                            .border_1()
                            .border_color(theme::hairline())
                            .bg(theme::PORT)
                            .text_size(px(11.))
                            .text_color(theme::STENCIL_DIM)
                            .child(shown),
                    ),
            )
    }

    fn render_activity(&self) -> Option<Div> {
        if self.activity.is_empty() {
            return None;
        }
        Some(
            v_flex()
                .gap_3()
                .child(common::section_title(t("Recent MCP commands")))
                .child(v_flex().gap_1().children(self.activity.iter().map(|entry| {
                    let (label, tone) = match entry.error {
                        None => (t("Done"), theme::GREEN_TEXT),
                        Some(_) => (t("Failed"), theme::RED_TEXT),
                    };
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(div().text_color(tone).child(label))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(theme::STENCIL_MUTE)
                                .child(entry.connection.clone()),
                        )
                        .child(div().flex_shrink_0().child(entry.tool.clone()))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme::STENCIL_DIM)
                                .child(entry.detail.replace('\n', " ")),
                        )
                }))),
        )
    }
}

impl Render for McpView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let saving = self.saving;
        let port_changed = self.port.read(cx).value().trim() != self.status.port.to_string();
        let running = self.status.running.clone();
        let row = |label: SharedString, control: AnyElement| {
            h_flex()
                .gap_3()
                .text_sm()
                .child(
                    div()
                        .w(px(128.))
                        .flex_shrink_0()
                        .text_color(theme::STENCIL_MUTE)
                        .child(label),
                )
                .child(control)
        };

        let mut rows = v_flex().gap_3().child(row(
            t("Port"),
            h_flex()
                .gap_2()
                .child(
                    div()
                        .w(px(112.))
                        .child(Input::new(&self.port).small().disabled(saving)),
                )
                .when(port_changed, |port| {
                    port.child(
                        Button::new("apply-port")
                            .outline()
                            .small()
                            .label(t("Apply"))
                            .disabled(saving)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.apply_port(window, cx)),
                            ),
                    )
                })
                .into_any_element(),
        ));
        if let Some((url, token)) = &running {
            rows = rows
                .child(row(
                    "URL".into(),
                    h_flex()
                        .gap_2()
                        .child(div().text_xs().child(url.clone()))
                        .child(self.copy_button("copy-url", url.clone(), cx))
                        .into_any_element(),
                ))
                .child(row(
                    "Token".into(),
                    self.render_token(token, cx).into_any_element(),
                ));
        }

        let error = self
            .error
            .clone()
            .or_else(|| self.status.error.clone().map(SharedString::from));
        let mut body = v_flex()
            .gap_6()
            .child(rows)
            .children(error.map(|error| div().text_xs().text_color(theme::RED_TEXT).child(error)));
        if let Some((url, token)) = &running {
            body = body.child(self.render_client(url, token, cx)).child(
                v_flex()
                    .gap_3()
                    .child(common::section_title(t("Tools")))
                    .child(v_flex().gap_2().children(TOOLS.map(|(name, description)| {
                        h_flex()
                            .gap_3()
                            .child(div().w(px(160.)).flex_shrink_0().text_xs().child(name))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::STENCIL_DIM)
                                    .child(t(description)),
                            )
                    }))),
            );
        }
        body = body.children(self.render_activity());

        common::page("MCP".into(), div(), common::page_body("mcp").child(body))
    }
}
