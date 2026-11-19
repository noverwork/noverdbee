//! A query plan: its steps as a tree with their costs beside them, or the plan
//! as the database gave it.

use std::collections::HashSet;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::db::plan::{Plan, PlanNode};
use crate::ui::i18n::{t, tf};
use crate::ui::icons::lucide;
use crate::ui::theme;

/// Width of each number column.
const NUMBER_WIDTH: Pixels = px(96.);

pub struct QueryPlan {
    plan: Plan,
    /// Nodes whose children are hidden, by their place in a depth-first walk.
    collapsed: HashSet<usize>,
    raw: bool,
}

/// A visible row: the node's depth-first index, its depth, and the node.
type Row<'a> = (usize, usize, &'a PlanNode);

/// The rows shown, skipping the children of collapsed nodes. Every node gets
/// an index, shown or not, so indexes stay put as rows open and close.
fn visible<'a>(
    nodes: &'a [PlanNode],
    depth: usize,
    hidden: bool,
    collapsed: &HashSet<usize>,
    next: &mut usize,
    rows: &mut Vec<Row<'a>>,
) {
    for node in nodes {
        let index = *next;
        *next += 1;
        if !hidden {
            rows.push((index, depth, node));
        }
        let hide = hidden || collapsed.contains(&index);
        visible(&node.children, depth + 1, hide, collapsed, next, rows);
    }
}

/// `12`, `0.42`
fn number(value: Option<f64>) -> SharedString {
    match value {
        Some(value) if value.fract() == 0.0 => format!("{value:.0}").into(),
        Some(value) => format!("{value:.2}").into(),
        None => SharedString::default(),
    }
}

impl QueryPlan {
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            collapsed: HashSet::new(),
            raw: false,
        }
    }

    /// `Planning 0.10 ms · Execution 0.50 ms`, for the tab summary.
    pub fn summary(&self) -> SharedString {
        let ms = |value: f64| format!("{value:.2}");
        match (self.plan.planning_ms, self.plan.execution_ms) {
            (Some(planning), Some(execution)) => tf(
                "Planning {planning} ms · Execution {execution} ms",
                &[("planning", &ms(planning)), ("execution", &ms(execution))],
            ),
            (Some(planning), None) => tf("Planning {planning} ms", &[("planning", &ms(planning))]),
            _ => t("Query plan"),
        }
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&index) {
            self.collapsed.insert(index);
        }
        cx.notify();
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let raw = self.plan.raw.clone();
        h_flex()
            .h_7()
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(theme::RIB)
            .child(
                Button::new("plan-tree")
                    .ghost()
                    .xsmall()
                    .icon(lucide("list-tree"))
                    .label(t("Tree"))
                    .selected(!self.raw)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.raw = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("plan-raw")
                    .ghost()
                    .xsmall()
                    .icon(lucide("file-code"))
                    .label(t("Raw"))
                    .selected(self.raw)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.raw = true;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("plan-copy")
                    .ghost()
                    .xsmall()
                    .icon(lucide("copy"))
                    .tooltip(t("Copy the plan as the database gave it"))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(raw.clone()))
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::STENCIL_MUTE)
                    .child(self.summary()),
            )
    }

    fn render_tree(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        visible(
            &self.plan.roots,
            0,
            false,
            &self.collapsed,
            &mut 0,
            &mut rows,
        );
        // SQLite gives one line per step: no detail, no numbers.
        let first = self.plan.roots.first();
        let costs = first.is_some_and(|node| node.total_cost.is_some());
        let analyzed = first.is_some_and(|node| node.actual_time.is_some());
        let mut headers = Vec::new();
        if costs {
            headers.extend([t("Cost"), t("Rows")]);
        }
        if analyzed {
            headers.extend([t("Actual ms"), t("Actual rows"), t("Loops")]);
        }

        let line = || {
            h_flex()
                .w_full()
                .min_w_0()
                .h_6()
                .px_2()
                .gap_3()
                .border_b_1()
                .border_color(theme::hairline())
        };
        let cell = |text: SharedString| {
            div()
                .w(NUMBER_WIDTH)
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(text)
        };
        let header = line()
            .text_size(px(11.))
            .text_color(theme::STENCIL_MUTE)
            .bg(theme::RAIL)
            .child(div().flex_1().min_w_0().child(t("Operation")))
            .when(costs, |row| {
                row.child(div().flex_1().min_w_0().child(t("Detail")))
            })
            .children(headers.into_iter().map(cell));

        let body = rows.into_iter().map(|(index, depth, node)| {
            let chevron = if node.children.is_empty() {
                div().size_3().flex_shrink_0().into_any_element()
            } else {
                let open = !self.collapsed.contains(&index);
                div()
                    .id(("plan-chevron", index))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(
                        lucide(if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .size_3()
                        .text_color(theme::STENCIL_MUTE),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(index, cx)))
                    .into_any_element()
            };
            let mut numbers = Vec::new();
            if costs {
                numbers.extend([number(node.total_cost), number(node.rows)]);
            }
            if analyzed {
                numbers.extend([
                    number(node.actual_time),
                    number(node.actual_rows),
                    number(node.loops),
                ]);
            }
            line()
                .text_size(px(12.))
                .text_color(theme::STENCIL_DIM)
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1p5()
                        .pl(px(14. * depth as f32))
                        .child(chevron)
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme::STENCIL)
                                .child(node.operation.clone()),
                        ),
                )
                .when(costs, |row| {
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::STENCIL_MUTE)
                            .child(node.detail.clone()),
                    )
                })
                .children(numbers.into_iter().map(cell))
        });

        v_flex()
            .id("plan-tree")
            .size_full()
            .overflow_y_scroll()
            .child(header)
            .children(body)
    }
}

impl Render for QueryPlan {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.raw {
            div()
                .id("plan-raw")
                .size_full()
                .overflow_scroll()
                .p_3()
                .text_size(px(12.))
                .text_color(theme::STENCIL_DIM)
                .child(self.plan.raw.clone())
                .into_any_element()
        } else {
            self.render_tree(cx).into_any_element()
        };
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::visible;
    use crate::db::plan::PlanNode;

    fn node(name: &str, children: Vec<PlanNode>) -> PlanNode {
        PlanNode {
            operation: name.to_string(),
            children,
            ..PlanNode::default()
        }
    }

    #[test]
    fn a_collapsed_node_hides_its_subtree_and_keeps_later_indexes() {
        let roots = vec![
            node(
                "join",
                vec![
                    node("scan a", vec![node("inner", vec![])]),
                    node("scan b", vec![]),
                ],
            ),
            node("next", vec![]),
        ];
        let mut rows = Vec::new();
        visible(&roots, 0, false, &HashSet::from([1]), &mut 0, &mut rows);
        let shown: Vec<_> = rows
            .iter()
            .map(|(index, depth, node)| (*index, *depth, node.operation.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (0, 0, "join"),
                (1, 1, "scan a"),
                (3, 1, "scan b"),
                (4, 0, "next")
            ]
        );
    }
}
