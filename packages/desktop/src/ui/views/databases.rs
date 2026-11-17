//! The databases page. There is nothing to list yet.

use gpui_kit::*;

use crate::ui::i18n::t;
use crate::ui::views::common;

pub struct DatabasesView;

impl Render for DatabasesView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        common::page(
            t("Databases"),
            div(),
            common::page_body("databases").child(common::empty_state(t("No connections yet"))),
        )
    }
}
