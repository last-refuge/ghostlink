//! Manual transport probe. Observe updates without timer polling or recording input.
use super::*;

pub(crate) fn measure(name: &str, session: &Session, cases: impl Iterator<Item = (Input, String)>) {
    let updates = session.updates();
    let (tx, rx) = mpsc::sync_channel(1);
    let observer = thread::spawn(move || {
        while let Ok(update) = updates.recv_blocking() {
            if tx.send(update).is_err() {
                break;
            }
        }
    });
    let mut samples = Vec::new();
    for (input, marker) in cases {
        let start = Instant::now();
        session.send(input).unwrap();
        loop {
            let update = rx
                .recv_timeout(Duration::from_secs(5).saturating_sub(start.elapsed()))
                .expect("Echo probe timed out");
            assert!(!matches!(update.state, State::Failed(_)), "{update:?}");
            if update
                .screen
                .as_ref()
                .is_some_and(|s| s.text().lines().any(|line| line == marker))
            {
                samples.push(start.elapsed());
                break;
            }
        }
    }
    samples.sort();
    eprintln!(
        "{name} send-to-snapshot: p50={:.2}ms p95={:.2}ms max={:.2}ms",
        samples[samples.len() / 2].as_secs_f64() * 1000.,
        samples[samples.len() * 95 / 100].as_secs_f64() * 1000.,
        samples.last().unwrap().as_secs_f64() * 1000.
    );
    drop(rx);
    // The observer stops when this session closes, or on its next update.
    drop(observer);
}
