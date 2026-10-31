use crate::field::Field;
use gpui::{
    App, Context, Entity, EventEmitter, Focusable, FontWeight, KeyDownEvent, Window, div,
    prelude::*, px, rgb,
};
use session::SshProfile;

pub enum ConnectEvent {
    Submit { profile: SshProfile },
    Cancel,
}
pub struct ConnectForm {
    destination: Entity<Field>,
    error: Option<String>,
}
impl EventEmitter<ConnectEvent> for ConnectForm {}
impl ConnectForm {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            destination: cx.new(|cx| Field::new("", false, cx)),
            error: None,
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &App) {
        window.focus(&self.destination.focus_handle(cx));
    }
    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }
    fn submit(&mut self, cx: &mut Context<Self>) {
        match SshProfile::from_destination(self.destination.read(cx).value()) {
            Ok(profile) => cx.emit(ConnectEvent::Submit { profile }),
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
}
impl Render for ConnectForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(520.)).p_6().flex().flex_col().gap_3().rounded_lg().border_1().border_color(rgb(0x3a4549)).bg(rgb(0x1b2023)).shadow_lg()
            .on_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|{
                match event.keystroke.key.as_str(){"escape"=>cx.emit(ConnectEvent::Cancel),"enter"=>this.submit(cx),"tab"=>this.focus(window,cx),_=>return,}
                window.prevent_default();cx.stop_propagation();
            }))
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Connect with SSH"))
            .child(div().text_xs().text_color(rgb(0x8e9b9e)).child("Host or SSH alias"))
            .child(self.destination.clone())
            .child(div().text_xs().text_color(rgb(0x8e9b9e)).child("server, user@server, or user@server:2222"))
            .child(div().text_color(rgb(0xa8b6b7)).child("Uses your SSH config and existing keys. You'll be asked for a passphrase or password only when needed."))
            .when_some(self.error.clone(),|root,error|root.child(div().text_color(rgb(0xd7b77d)).child(error)))
            .child(div().flex().justify_end().gap_3()
                .child(div().id("cancel-connect").px_4().py_2().cursor_pointer().on_click(cx.listener(|_,_,_,cx|cx.emit(ConnectEvent::Cancel))).child("Cancel"))
                .child(div().id("submit-connect").px_4().py_2().rounded_sm().cursor_pointer().bg(rgb(0xa8d8c0)).text_color(rgb(0x111517)).on_click(cx.listener(|this,_,_,cx|this.submit(cx))).child("Connect")))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use session::SshAuth;
    #[gpui::test]
    fn one_destination_submits_automatic_identity_discovery(cx: &mut TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        let window = cx.add_window(|window, cx| {
            let mut form = ConnectForm::new(cx);
            form.destination = cx.new(|cx| Field::new("deploy@example.org:2222", false, cx));
            form.focus(window, cx);
            form
        });
        let submitted = Rc::new(RefCell::new(None));
        let captured = submitted.clone();
        let form = window.root(cx).unwrap();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&form, move |_, event, _| {
                if let ConnectEvent::Submit { profile } = event {
                    *captured.borrow_mut() = Some(profile.clone());
                }
            })
        });
        cx.simulate_keystrokes(*window, "enter");
        let submitted = submitted.borrow();
        let profile = submitted.as_ref().unwrap();
        assert_eq!(profile.host, "example.org");
        assert_eq!(profile.username, "deploy");
        assert_eq!(profile.port, 2222);
        assert!(matches!(profile.auth, SshAuth::Automatic { .. }));
    }
    #[gpui::test]
    fn destination_validation_and_keyboard_entry(cx: &mut TestAppContext) {
        let window = cx.add_window(|window, cx| {
            let form = ConnectForm::new(cx);
            form.focus(window, cx);
            form
        });
        cx.simulate_keystrokes(*window, "enter");
        window
            .update(cx, |form, _, _| assert!(form.error.is_some()))
            .unwrap();
        cx.simulate_keystrokes(*window, "p r o d tab");
        window
            .update(cx, |form, window, cx| {
                assert_eq!(form.destination.read(cx).value(), "prod");
                assert!(form.destination.focus_handle(cx).is_focused(window));
            })
            .unwrap();
    }
}
