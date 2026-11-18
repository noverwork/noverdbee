//! Building blocks every page shares: the page frame, form sections, fields,
//! banners, and the delete confirmation.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenu, ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::select::{SelectItem, SelectState};
use gpui_kit::component::{
    h_flex, v_flex, Disableable as _, IndexPath, Sizable as _, WindowExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::theme;

/// The frame of a list page: a title row with actions, then scrolling content.
pub fn page(title: SharedString, actions: impl IntoElement, body: impl IntoElement) -> Div {
    v_flex()
        .size_full()
        .px_8()
        .py_6()
        .bg(theme::PORT)
        .text_color(theme::STENCIL)
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .p_6()
                .child(
                    h_flex()
                        .items_start()
                        .justify_between()
                        .gap_4()
                        .pb_5()
                        .border_b_1()
                        .border_color(theme::hairline())
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(h_flex().gap_2().child(actions)),
                )
                .child(body),
        )
}

/// The scrolling area under a page's title row.
pub fn page_body(id: &'static str) -> Stateful<Div> {
    div().id(id).mt_6().flex_1().min_h_0().overflow_y_scroll()
}

/// The body of a form drawer: scrolling fields, then Cancel and Save.
pub fn form_page(
    error: Option<SharedString>,
    fields: impl IntoElement,
    footer: impl IntoElement,
) -> Div {
    // The drawer body scrolls and so gives its content no height; filling it
    // absolutely keeps Cancel and Save pinned while only the fields scroll.
    v_flex()
        .absolute()
        .inset_0()
        .text_color(theme::STENCIL)
        .child(
            div()
                .id("form-fields")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    v_flex()
                        .gap_5()
                        // Level with the drawer's title and close button.
                        .px_4()
                        .py_4()
                        .children(error.map(error_banner))
                        .child(fields),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .justify_end()
                .gap_2()
                .px_4()
                .py_4()
                .border_t_1()
                .border_color(theme::hairline())
                .child(footer),
        )
}

/// Cancel and submit, for [`form_page`]'s footer.
pub fn form_buttons(
    submit_label: SharedString,
    busy: bool,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_submit: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    h_flex()
        .gap_2()
        .child(
            Button::new("form-cancel")
                .ghost()
                .label(t("Cancel"))
                .disabled(busy)
                .on_click(on_cancel),
        )
        .child(
            Button::new("form-submit")
                .primary()
                .label(submit_label)
                .loading(busy)
                .disabled(busy)
                .on_click(on_submit),
        )
}

/// A group of fields under a title, with a rule over it. Add the fields with
/// `child`. Pass no title for the first group, which the drawer title already
/// names; it gets no rule either.
#[derive(IntoElement)]
pub struct FormSection {
    title: Option<SharedString>,
    hint: Option<SharedString>,
    fields: Vec<AnyElement>,
}

pub fn form_section(
    title: impl Into<Option<SharedString>>,
    hint: Option<SharedString>,
) -> FormSection {
    FormSection {
        title: title.into(),
        hint,
        fields: Vec::new(),
    }
}

impl ParentElement for FormSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.fields.extend(elements);
    }
}

impl RenderOnce for FormSection {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let fields = v_flex().gap_4().children(self.fields);
        let Some(title) = self.title else {
            return fields;
        };
        v_flex()
            .gap_4()
            .pt_5()
            .border_t_1()
            .border_color(theme::hairline())
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child(title))
                    .children(
                        self.hint.map(|hint| {
                            div().text_xs().text_color(theme::STENCIL_MUTE).child(hint)
                        }),
                    ),
            )
            .child(fields)
    }
}

/// Two fields side by side, each taking half the row.
pub fn field_pair(left: impl IntoElement, right: impl IntoElement) -> Div {
    h_flex()
        .items_start()
        .gap_3()
        .child(div().flex_1().min_w_0().child(left))
        .child(div().flex_1().min_w_0().child(right))
}

/// The 40px bordered square that holds a row's or section's icon.
pub fn icon_box(icon: &'static str) -> Div {
    div()
        .size_10()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS)
        .border_1()
        .border_color(theme::hairline())
        .text_color(theme::STENCIL_DIM)
        .child(lucide(icon).size_5())
}

/// A label over a control, with an optional validation message under it.
pub fn field(label: SharedString, control: impl IntoElement, error: Option<SharedString>) -> Div {
    v_flex()
        .gap_1p5()
        .child(div().text_xs().text_color(theme::STENCIL_DIM).child(label))
        .child(control)
        .children(error.map(|error| div().text_xs().text_color(theme::RED_TEXT).child(error)))
}

/// The small uppercase heading over a group of rows.
pub fn section_title(title: SharedString) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::STENCIL_MUTE)
        .child(SharedString::from(title.to_uppercase()))
}

pub fn error_banner(error: SharedString) -> Div {
    div()
        .px_4()
        .py_3()
        .rounded(theme::RADIUS)
        .border_1()
        .border_color(theme::SEAL.opacity(0.6))
        .bg(theme::SEAL.opacity(0.1))
        .text_sm()
        .text_color(theme::RED_TEXT)
        .child(error)
}

/// The dashed box shown when a list has nothing in it.
pub fn empty_state(message: SharedString) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_h(px(256.))
        .px_4()
        .py_8()
        .rounded(theme::RADIUS)
        .border_1()
        .border_dashed()
        .border_color(theme::hairline())
        .text_sm()
        .text_color(theme::STENCIL_MUTE)
        .child(message)
}

/// A card in a list: bordered, and brighter under the pointer.
pub fn card(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .px_4()
        .py_4()
        .rounded(theme::RADIUS)
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::RIB)
        .hover(|style| {
            style
                .border_color(theme::STENCIL.opacity(0.2))
                .bg(theme::RIB_HOVER)
        })
}

/// A page listing saved items as cards, with a search box and a button to
/// add one. `cards` are the items matching the search; `empty` says why
/// there are none.
pub fn list_page(
    title: SharedString,
    search: &Entity<InputState>,
    add_label: SharedString,
    on_add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    error: Option<SharedString>,
    empty: SharedString,
    cards: Vec<AnyElement>,
) -> Div {
    let body = if cards.is_empty() {
        empty_state(empty).into_any_element()
    } else {
        div()
            .grid()
            .grid_cols(3)
            .gap_3()
            .children(cards)
            .into_any_element()
    };
    page(
        title,
        h_flex()
            .gap_2()
            .child(
                div().w(px(192.)).child(
                    Input::new(search)
                        .small()
                        .prefix(lucide("search").xsmall().text_color(theme::STENCIL_FAINT)),
                ),
            )
            .child(
                Button::new("add-item")
                    .primary()
                    .small()
                    .icon(lucide("plus"))
                    .label(add_label)
                    .on_click(on_add),
            ),
        page_body("items")
            .children(error.map(|error| error_banner(error).mb_4()))
            .child(body),
    )
}

/// The search box of a [`list_page`]; the page redraws as the text changes.
pub fn new_search<V: 'static>(
    placeholder: SharedString,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (Entity<InputState>, Subscription) {
    let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    let changed = cx.observe(&search, |_, _, cx| cx.notify());
    (search, changed)
}

/// Whether `query`, as typed into a search box, is in any of `texts`.
pub fn matches(query: &str, texts: &[&str]) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || texts
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
}

/// Shown instead of a form whose item no longer exists.
pub fn not_found(
    title: SharedString,
    back_label: SharedString,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_5()
        .child(lucide("circle-alert").size_7().text_color(theme::RED_TEXT))
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            Button::new("back")
                .primary()
                .label(back_label)
                .on_click(on_back),
        )
}

/// A card for one saved item: its icon, name and detail lines. Clicking it
/// opens the item; the delete button shows under the pointer, and a right
/// click offers both.
pub fn item_card(
    id: &str,
    icon: &'static str,
    name: &str,
    details: Vec<SharedString>,
    on_open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_delete: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> ContextMenu<Stateful<Div>> {
    type Handler = std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
    let (on_open, on_delete): (Handler, Handler) =
        (std::rc::Rc::new(on_open), std::rc::Rc::new(on_delete));
    let (click_open, click_delete) = (on_open.clone(), on_delete.clone());
    let menu_label = SharedString::from(name.to_string());
    let group: SharedString = format!("item-{id}").into();
    card(group.clone())
        .group(group.clone())
        .cursor_pointer()
        .on_click(move |event, window, cx| click_open(event, window, cx))
        .child(
            h_flex()
                .items_start()
                .gap_3()
                .child(icon_box(icon))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.to_string()),
                        )
                        .children(details.into_iter().map(|detail| {
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(theme::STENCIL_MUTE)
                                .child(detail)
                        })),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .invisible()
                        .group_hover(group, |style| style.visible())
                        .child(
                            Button::new(SharedString::from(format!("delete-{id}")))
                                .ghost()
                                .small()
                                .icon(lucide("trash"))
                                .tooltip(tf("Delete {name}", &[("name", name)]))
                                .on_click(move |event, window, cx| {
                                    cx.stop_propagation();
                                    click_delete(event, window, cx);
                                }),
                        ),
                ),
        )
        .context_menu(move |menu, _, _| {
            let (on_open, on_delete) = (on_open.clone(), on_delete.clone());
            menu.label(menu_label.clone())
                .separator()
                .item(
                    PopupMenuItem::new(t("Edit"))
                        .icon(lucide("pencil"))
                        .on_click(move |event, window, cx| on_open(event, window, cx)),
                )
                .separator()
                .item(
                    PopupMenuItem::new(t("Delete"))
                        .icon(lucide("trash"))
                        .on_click(move |event, window, cx| on_delete(event, window, cx)),
                )
        })
}

/// Asks before deleting `item_name`; `on_confirm` runs when the user agrees.
pub fn confirm_delete(
    title: SharedString,
    description: SharedString,
    item_name: Option<SharedString>,
    confirm_label: SharedString,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_confirm = std::rc::Rc::new(on_confirm);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let on_confirm = on_confirm.clone();
        let description = v_flex().gap_3().child(description.clone()).when_some(
            item_name.clone(),
            |body, name| {
                body.child(
                    v_flex()
                        .gap_1()
                        .px_4()
                        .py_3()
                        .rounded(theme::RADIUS)
                        .border_1()
                        .border_color(theme::hairline())
                        .bg(theme::RIB)
                        .child(section_title(t("Target")))
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(theme::STENCIL)
                                .child(name),
                        ),
                )
            },
        );
        alert
            .title(title.clone())
            .description(description)
            .confirm()
            .ok_text(confirm_label.clone())
            .ok_variant(ButtonVariant::Danger)
            .cancel_text(t("Cancel"))
            .on_ok(move |_, window, cx| {
                on_confirm(window, cx);
                true
            })
    });
}

/// One option in a [`ChoiceSelect`].
#[derive(Clone)]
pub struct Choice {
    pub value: String,
    pub label: SharedString,
}

impl Choice {
    pub fn new(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for Choice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

/// A dropdown over string values. Render with `Select::new(&state)`; read with
/// [`selected`].
pub type ChoiceSelect = SelectState<Vec<Choice>>;

pub fn new_select(
    choices: Vec<Choice>,
    selected: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ChoiceSelect> {
    let index = choices.iter().position(|choice| choice.value == selected);
    cx.new(|cx| SelectState::new(choices, index.map(IndexPath::new), window, cx))
}

pub fn selected(select: &Entity<ChoiceSelect>, cx: &App) -> Option<String> {
    select.read(cx).selected_value().cloned()
}
