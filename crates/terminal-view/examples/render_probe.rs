//! Native Windows renderer probe: a dense viewport with small periodic updates.
//! Run with GHOSTLINK_PROFILE=1; reports timings only, never terminal contents.
use gpui::{AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use session::LocalProfile;
use std::time::Duration;
use terminal_view::TerminalView;

fn main() {
    Application::new().run(|cx| {
        let profile = LocalProfile {
            args: vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(),
                "$e=[char]27; [Console]::Write(\"$e[2J$e[H\"); 1..42 | ForEach-Object { [Console]::Write(('row {0:D2}  abcdefghijklmnopqrstuvwxyz 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ --- terminal render probe ---' -f $_)+\"`r`n\") }; 1..80 | ForEach-Object { [Console]::Write(\"$e[30;1Hupdate $_     \"); Start-Sleep -Milliseconds 40 }".into()],
            ..LocalProfile::default()
        };
        let bounds = Bounds::centered(None, size(px(1500.), px(1000.)), cx);
        cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        }, |_, cx| cx.new(|cx| TerminalView::new(profile, "Render probe".into(), cx))).unwrap();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            let _ = cx.update(|cx| cx.quit());
        }).detach();
        cx.activate(true);
    });
}
