//! Opt-in timings for developer probes. Never records terminal text or keystrokes.
use std::{sync::OnceLock, time::Instant};

pub(crate) struct Timer(&'static str, Option<Instant>);
impl Timer {
    pub fn start(name: &'static str) -> Self {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        Self(
            name,
            ENABLED
                .get_or_init(|| std::env::var_os("GHOSTLINK_PROFILE").is_some())
                .then(Instant::now),
        )
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(start) = self.1 {
            eprintln!("{} {:.3}", self.0, start.elapsed().as_secs_f64() * 1000.);
        }
    }
}
