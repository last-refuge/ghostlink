use super::*;
use std::collections::BTreeMap;
impl ConnectForm {
    pub(super) fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let mut groups = BTreeMap::<String, Vec<SavedConnection>>::new();
        for host in self.visible(cx) {
            groups
                .entry(host.options.group.clone())
                .or_default()
                .push(host.clone());
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x9aada4))
                    .child("Find a host"),
            )
            .when(self.entries.is_empty() && !self.load_failed, |list| {
                list.child(empty_connections_artwork())
            })
            .child(self.search.clone())
            .child(
                action("add-connection", "+ New host", false).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.request(Pending::New, window, cx);
                    },
                )),
            )
            .child(
                div()
                    .id("saved-connections")
                    .debug_selector(|| "saved-connections".into())
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(groups.into_iter().map(|(group, hosts)| {
                        let collapsed = self.collapsed.contains(&group);
                        let title = if group.is_empty() {
                            "Ungrouped".into()
                        } else {
                            group.clone()
                        };
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .id(gpui::SharedString::from(format!("group-{group}")))
                                    .px_2()
                                    .py_2()
                                    .text_xs()
                                    .text_color(rgb(0x9aada4))
                                    .cursor_pointer()
                                    .child(format!(
                                        "{} {title} ({})",
                                        if collapsed { "▸" } else { "▾" },
                                        hosts.len()
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.collapsed.remove(&group) {
                                            this.collapsed.insert(group.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .when(!collapsed, |list| {
                                list.children(hosts.into_iter().map(|host| {
                                    let id = host.id;
                                    div()
                                        .id(("connection", id as usize))
                                        .px_3()
                                        .py_3()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .bg(rgb(if self.selected == Some(id) {
                                            0x293e36
                                        } else {
                                            0x171c1f
                                        }))
                                        .hover(|style| style.bg(rgb(0x30463c)))
                                        .on_click(cx.listener(
                                            move |this, event: &ClickEvent, window, cx| {
                                                this.request(
                                                    if event.click_count() >= 2 {
                                                        Pending::Connect(id)
                                                    } else {
                                                        Pending::Select(id)
                                                    },
                                                    window,
                                                    cx,
                                                );
                                            },
                                        ))
                                        .child(
                                            div()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(host.label.clone()),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_xs()
                                                .text_color(rgb(0x9aada4))
                                                .child(host.destination()),
                                        )
                                        .when(!host.options.tags.is_empty(), |row| {
                                            row.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(0xa8d8c0))
                                                    .child(host.options.tags.join(" · ")),
                                            )
                                        })
                                }))
                            })
                    })),
            )
            .when(self.visible(cx).is_empty(), |list| {
                list.child(div().p_3().text_color(rgb(0x9aada4)).child(
                    if self.entries.is_empty() {
                        "Add your first SSH host."
                    } else {
                        "No hosts match your search."
                    },
                ))
            })
    }
}

pub(super) fn empty_connections_artwork() -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .py_2()
        .flex_shrink_0()
        .child(
            gpui::img(crate::brand::illustration())
                .id("empty-connections-illustration")
                .debug_selector(|| "empty-connections-illustration".into())
                .w_full()
                .h(px(132.))
                .object_fit(gpui::ObjectFit::Contain),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(0xdce3df))
                .child("Your connections start here"),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x9aada4))
                .child("Save your first host to keep it close."),
        )
}
