//! Live reload: a recursive file watcher on the board folder. Bursts of
//! events (an editor's save, a `git checkout`) collapse into one signal.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Keeps the watcher alive; dropping it stops the events.
pub struct BoardWatcher {
    _watcher: RecommendedWatcher,
}

/// Starts watching `root`. The receiver yields the changed markdown paths,
/// debounced to one batch per quiet period.
pub fn watch(root: &Path) -> notify::Result<(BoardWatcher, UnboundedReceiver<Vec<PathBuf>>)> {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else {
            return;
        };
        if matches!(ev.kind, EventKind::Access(_)) {
            return;
        }
        for p in ev.paths {
            let md = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"));
            // A folder event (rename / delete of a directory) reloads too.
            if md || p.extension().is_none() {
                let _ = raw_tx.send(p);
            }
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    let (tx, rx) = unbounded();
    std::thread::Builder::new()
        .name("kuzgun-watch".into())
        .spawn(move || {
            while let Ok(first) = raw_rx.recv() {
                let mut batch = vec![first];
                // Quiet period: keep collecting until 120 ms pass with no event.
                while let Ok(p) = raw_rx.recv_timeout(Duration::from_millis(120)) {
                    if !batch.contains(&p) {
                        batch.push(p);
                    }
                }
                if tx.unbounded_send(batch).is_err() {
                    return;
                }
            }
        })
        .ok();
    Ok((BoardWatcher { _watcher: watcher }, rx))
}
