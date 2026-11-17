//! The window: sidebar, page header, and the current page.

use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::i18n::t;
use crate::ui::icons::lucide;
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views;
use crate::ui::views::workspace::Workspace;

actions!(noverdbee, [OpenSettings]);

pub fn bind_keys(cx: &mut App) {
    // Cmd on macOS; Ctrl elsewhere.
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([KeyBinding::new(
        format!("{modifier}-,").leak(),
        OpenSettings,
        None,
    )]);
}

pub struct Shell {
    store: Entity<Store>,
    /// The Databases page, kept so open connections and tabs outlive a visit
    /// to another page. Built on first use, once the saved locale is known,
    /// because input placeholders are translated when they are created.
    workspace: Option<Entity<Workspace>>,
    /// The page for the current route; rebuilt when the route changes.
    page: Option<(Route, AnyView)>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let changes = cx.observe(&store, |_, _, cx| cx.notify());
        views::start_update_check(window, cx);
        Self {
            workspace: None,
            store,
            page: None,
            _subscriptions: vec![changes],
        }
    }

    pub fn workspace(&self) -> Option<&Entity<Workspace>> {
        self.workspace.as_ref()
    }

    fn page(&mut self, route: &Route, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        if *route == Route::Databases {
            return self
                .workspace
                .get_or_insert_with(|| cx.new(|cx| Workspace::new(window, cx)))
                .clone()
                .into();
        }
        match &self.page {
            Some((shown, view)) if shown == route => view.clone(),
            _ => {
                let view = views::build(route, window, cx);
                self.page = Some((route.clone(), view.clone()));
                view
            }
        }
    }

    // --- Sidebar ------------------------------------------------------------

    fn nav_item(
        &self,
        id: &'static str,
        label: SharedString,
        icon: &'static str,
        route: Route,
        active: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let store = self.store.clone();
        rail_button(id, active)
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .on_click(cx.listener(move |_, _, _, cx| {
                store.update(cx, |store, cx| store.navigate(route.clone(), cx));
            }))
            .child(lucide(icon).size_3p5())
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let route = store.route.clone();
        let store_handle = self.store.clone();
        let active = match route {
            Route::Databases | Route::ConnectionForm(_) | Route::NewConnectionIn(_) => "databases",
            Route::Mcp => "mcp",
            Route::Settings => "settings",
        };

        v_flex()
            .w(px(56.))
            .h_full()
            .flex_shrink_0()
            .bg(theme::RAIL)
            .border_r_1()
            .border_color(theme::RIB)
            .child(
                h_flex().justify_center().px_2().pt_3().pb_2().child(
                    div()
                        .id("home")
                        .size_10()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS)
                        .border_1()
                        .border_color(theme::RIB)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::wash()))
                        .tooltip(|window, cx| Tooltip::new(t("Go to Databases")).build(window, cx))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store
                                .update(cx, |store, cx| store.navigate(Route::Databases, cx));
                        }))
                        .child(img("icon.png").size_7().rounded(theme::RADIUS)),
                ),
            )
            .child(
                v_flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_3()
                    .child(self.nav_item(
                        "nav-databases",
                        t("Databases"),
                        "database",
                        Route::Databases,
                        active == "databases",
                        cx,
                    ))
                    .child(self.nav_item(
                        "nav-mcp",
                        t("MCP"),
                        "plug",
                        Route::Mcp,
                        active == "mcp",
                        cx,
                    )),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .justify_center()
                    .p_2()
                    .border_t_1()
                    .border_color(theme::RIB)
                    .child(
                        rail_button("settings", active == "settings")
                            .tooltip(|window, cx| Tooltip::new(t("Settings")).build(window, cx))
                            .on_click(move |_, _, cx| {
                                store_handle
                                    .update(cx, |store, cx| store.navigate(Route::Settings, cx));
                            })
                            .child(lucide("settings").size_3p5()),
                    ),
            )
    }

    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let (label, icon) = match &self.store.read(cx).route {
            Route::Databases | Route::ConnectionForm(_) | Route::NewConnectionIn(_) => {
                (t("Databases"), "database")
            }
            Route::Mcp => (t("MCP"), "plug"),
            Route::Settings => (t("Settings"), "settings"),
        };
        h_flex()
            .h_9()
            .flex_shrink_0()
            .px_2()
            .bg(theme::RIB)
            .child(page_label(label, icon))
    }
}

/// The outlined cell at the left of the top bar that names the page.
pub fn page_label(label: SharedString, icon: &'static str) -> Div {
    h_flex()
        .h_7()
        .flex_shrink_0()
        .gap_2()
        .px_2p5()
        .rounded(theme::RADIUS)
        .border_1()
        .border_color(theme::STENCIL)
        .text_size(px(11.))
        .text_color(theme::STENCIL)
        .child(lucide(icon).size_3p5().text_color(theme::STENCIL_DIM))
        .child(SharedString::from(label.to_uppercase()))
}

/// A 40x28 icon button in the sidebar rail.
fn rail_button(id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h_7()
        .w_10()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS)
        .border_1()
        .cursor_pointer()
        .map(|button| {
            if active {
                button
                    .border_color(theme::STENCIL)
                    .text_color(theme::STENCIL)
            } else {
                button
                    .border_color(gpui_kit::transparent_black())
                    .text_color(theme::STENCIL_MUTE)
                    .hover(|style| {
                        style
                            .border_color(theme::hairline())
                            .bg(theme::wash())
                            .text_color(theme::STENCIL)
                    })
            }
        })
}

fn centered() -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .p_8()
        .bg(theme::PORT)
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (loading, load_error, route) = {
            let store = self.store.read(cx);
            (store.loading, store.load_error.clone(), store.route.clone())
        };
        if loading {
            return centered()
                .gap_4()
                .child(Spinner::new().large().color(theme::STENCIL.into()))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::STENCIL_MUTE)
                        .child(t("Loading...")),
                )
                .into_any_element();
        }
        if let Some(error) = load_error {
            return centered()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(px(672.))
                        .items_center()
                        .gap_4()
                        .p_8()
                        .rounded(theme::RADIUS)
                        .border_1()
                        .border_color(theme::hairline())
                        .bg(theme::RAIL)
                        .child(lucide("circle-alert").size_8().text_color(theme::RED_TEXT))
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t("Backend connection unavailable")),
                        )
                        .child(div().text_sm().text_color(theme::STENCIL_DIM).child(error))
                        .child(Button::new("retry").outline().label(t("Retry")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |store, cx| store.refresh(cx));
                            }),
                        )),
                )
                .into_any_element();
        }

        let content = self.page(&route, window, cx);

        h_flex()
            .size_full()
            .items_start()
            .bg(theme::PORT)
            .text_color(theme::STENCIL)
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.store
                    .update(cx, |store, cx| store.navigate(Route::Settings, cx));
            }))
            .child(self.render_sidebar(cx))
            .child(
                v_flex()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .when(route != Route::Databases, |column| {
                        column.child(self.render_header(cx))
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(content),
                    ),
            )
            .into_any_element()
    }
}
