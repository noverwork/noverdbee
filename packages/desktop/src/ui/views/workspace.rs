//! The Databases page: the navigator beside the work area. The shell keeps
//! one for the life of the window, so what is open survives a visit to
//! another page.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::ui::i18n::t;
use crate::ui::store::{store, Route, Store};
use crate::ui::theme;
use crate::ui::views::navigator::Navigator;

pub struct Workspace {
    store: Entity<Store>,
    navigator: Entity<Navigator>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = store(cx);
        let navigator = cx.new(|cx| Navigator::new(window, cx));
        let _subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            navigator,
            _subscriptions,
        }
    }

    pub fn navigator(&self) -> &Entity<Navigator> {
        &self.navigator
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
        h_resizable("workspace")
            .child(
                resizable_panel()
                    .size(px(240.))
                    .size_range(px(200.)..px(400.))
                    .child(self.navigator.clone()),
            )
            .child(resizable_panel().child(self.render_empty(cx)))
    }
}
