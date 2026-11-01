use gpui::{
    AnyView, App, AppContext, Context, Render, SharedString, Window, div, prelude::*, px, rgb,
};

struct Hint(SharedString);
impl Render for Hint {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_sm()
            .border_1()
            .border_color(rgb(0x3a4549))
            .bg(rgb(0x222b2e))
            .text_color(rgb(0xdce3df))
            .font_family(ghostlink_platform::UI_FONT)
            .text_xs()
            .line_height(px(16.))
            .whitespace_nowrap()
            .shadow_sm()
            .child(self.0.clone())
    }
}
pub fn hint(text: impl Into<SharedString>, cx: &mut App) -> AnyView {
    let text = text.into();
    cx.new(|_| Hint(text)).into()
}
