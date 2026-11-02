use super::*;
use crate::host_options::{Proxy, ProxyKind};
use editor::field;

fn reaches(id: u64, target: u64, hosts: &[SavedConnection], seen: &mut Vec<u64>) -> bool {
    if id == target || seen.len() > 8 || seen.contains(&id) {
        return true;
    }
    let Some(host) = hosts.iter().find(|host| host.id == id) else {
        return true;
    };
    seen.push(id);
    let found = host
        .options
        .jump_hosts
        .iter()
        .any(|hop| reaches(*hop, target, hosts, seen));
    seen.pop();
    found
}
impl ConnectForm {
    pub(super) fn render_routing(&self, cx: &mut Context<Self>) -> Div {
        let Some(editor) = &self.editor else {
            return div();
        };
        let query = editor.jump_search.read(cx).value().trim().to_lowercase();
        let candidates = self.entries.iter().filter(|host| {
            !editor.options.jump_hosts.contains(&host.id)
                && editor
                    .id
                    .is_none_or(|id| !reaches(host.id, id, &self.entries, &mut Vec::new()))
                && (host.label.to_lowercase().contains(&query)
                    || host.destination().to_lowercase().contains(&query))
        });
        div().flex().flex_col().gap_2()
            .child(div().font_weight(FontWeight::MEDIUM).child("Host chaining"))
            .child(div().text_xs().text_color(rgb(0x9aada4)).child("Connect through these saved hosts in order. Each host uses its own login and SSH options."))
            .children(editor.options.jump_hosts.iter().enumerate().map(|(index, id)| {
                let label = self.entries.iter().find(|host| host.id == *id).map(|host| host.label.clone()).unwrap_or_else(|| format!("Missing host #{id}"));
                div().id(("selected-jump", index)).flex().items_center().gap_1().py_1()
                    .child(div().flex_1().min_w_0().truncate().child(format!("{}. {label}", index + 1)))
                    .when(index > 0, |row| row.child(div().id("up").px_2().py_1().cursor_pointer().child("↑").on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &mut this.editor { editor.options.jump_hosts.swap(index, index - 1); } cx.notify(); }))))
                    .when(index + 1 < editor.options.jump_hosts.len(), |row| row.child(div().id("down").px_2().py_1().cursor_pointer().child("↓").on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &mut this.editor { editor.options.jump_hosts.swap(index, index + 1); } cx.notify(); }))))
                    .child(div().id("remove").px_2().py_1().cursor_pointer().child("×").on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &mut this.editor { editor.options.jump_hosts.remove(index); } cx.notify(); })))
            }))
            .child(field("jump-search", "Find a jump host to add", &editor.jump_search))
            .child(div().id("available-jump-hosts").max_h(px(130.)).overflow_y_scroll().children(candidates.map(|host| {
                let id = host.id;
                div().id(("add-jump", id as usize)).px_2().py_2().rounded_sm().cursor_pointer().hover(|style| style.bg(rgb(0x293438)))
                    .child(format!("+ {} · {}", host.label, host.destination()))
                    .on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &mut this.editor {
                        if editor.options.jump_hosts.len() >= 8 { this.error = Some("Use up to 8 jump hosts.".into()); }
                        else { editor.options.jump_hosts.push(id); editor.jump_search.update(cx, |field, cx| field.set_value("", cx)); }
                    } cx.notify(); }))
            })))
            .child(div().mt_2().font_weight(FontWeight::MEDIUM).child("Proxy"))
            .child(div().flex().flex_wrap().gap_1().children([(0, "None"), (1, "SOCKS5"), (2, "SOCKS4a"), (3, "HTTP CONNECT")].into_iter().map(|(mode, label)| {
                let selected = match &editor.options.proxy { None => mode == 0, Some(proxy) => matches!((&proxy.kind, mode), (ProxyKind::Socks5, 1) | (ProxyKind::Socks4, 2) | (ProxyKind::Http, 3)) };
                div().id(("proxy-mode", mode as usize)).px_2().py_2().rounded_sm().cursor_pointer().bg(rgb(if selected { 0x293e36 } else { 0x171c1f })).child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(editor) = &mut this.editor {
                            let default_port = if mode == 3 { 8080 } else { 1080 };
                            if editor.proxy_port.read(cx).value().trim().is_empty() { editor.proxy_port.update(cx, |field, cx| field.set_value(&default_port.to_string(), cx)); }
                            let remember = mode != 2 && editor.options.proxy.as_ref().is_some_and(|proxy| proxy.remember);
                            editor.options.proxy = match mode { 0 => None, _ => Some(Proxy { kind: match mode { 1 => ProxyKind::Socks5, 2 => ProxyKind::Socks4, _ => ProxyKind::Http }, host: editor.proxy_host.read(cx).value().into(), port: editor.proxy_port.read(cx).value().parse().unwrap_or(default_port), username: editor.proxy_username.read(cx).value().into(), remember }) };
                        }
                        this.focus_editor(window, cx); cx.notify();
                    }))
            })))
            .when_some(editor.options.proxy.as_ref(), |root, proxy| {
                root.child(field("proxy-host", "Proxy address", &editor.proxy_host))
                    .child(field("proxy-port", "Proxy port", &editor.proxy_port))
                    .child(field("proxy-username", if proxy.kind == ProxyKind::Socks4 { "SOCKS4 user ID (optional)" } else { "Proxy username (blank for anonymous)" }, &editor.proxy_username))
                    .when(proxy.kind != ProxyKind::Socks4, |root| root
                        .child(field("proxy-password", "Proxy password (blank to ask when needed)", &editor.proxy_password))
                        .child(div().id("remember-proxy-password").py_2().cursor_pointer().child(if proxy.remember { "☑ Remember proxy login in system keychain" } else { "☐ Remember proxy login in system keychain" })
                            .on_click(cx.listener(|this, _, _, cx| { if let Some(editor) = &mut this.editor && let Some(proxy) = &mut editor.options.proxy { proxy.remember = !proxy.remember; } cx.notify(); }))))
                    .when_some(editor.id.and_then(|id| secrets::last_error(id, SecretKind::Proxy)), |root, error| root.child(div().text_xs().text_color(rgb(0xd7b77d)).child(error)))
            })
    }
}
