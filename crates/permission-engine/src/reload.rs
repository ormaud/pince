//! Hot-reload watcher for policy files.
//!
//! Uses the `notify` crate to watch one or two policy paths. When either file
//! changes, the `PolicyEngine` is reloaded atomically.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::engine::PolicyEngine;

/// Start a background task that watches `paths` for modifications and reloads
/// the `PolicyEngine` on change.
///
/// Returns a guard handle; drop it to stop watching.
pub fn watch_and_reload(
    engine: Arc<PolicyEngine>,
    global_path: PathBuf,
    project_path: Option<PathBuf>,
) -> Result<ReloadGuard> {
    let (tx, mut rx) = mpsc::channel::<()>(4);

    // The notify watcher runs in a separate thread; debounce via a channel.
    let mut watcher: RecommendedWatcher = {
        let tx = tx.clone();
        notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    let _ = tx.blocking_send(());
                }
            }
        })?
    };

    // Watch the parent directories so we catch create/rename events.
    watch_path(&mut watcher, &global_path);
    if let Some(ref p) = project_path {
        watch_path(&mut watcher, p);
    }

    let global = global_path.clone();
    let project = project_path.clone();
    let engine_clone = engine.clone();

    tokio::spawn(async move {
        // Debounce: wait a bit after first notification before reloading.
        while rx.recv().await.is_some() {
            // Drain any additional events that arrived quickly.
            tokio::time::sleep(Duration::from_millis(200)).await;
            while rx.try_recv().is_ok() {}

            if let Err(e) = engine_clone.reload(&global, project.as_deref()).await {
                tracing::error!("policy reload failed: {e}");
            }
        }
    });

    Ok(ReloadGuard { _watcher: watcher })
}

fn watch_path(watcher: &mut RecommendedWatcher, path: &Path) {
    // Watch the parent directory if it exists; otherwise skip.
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else if let Some(parent) = path.parent() {
        parent.to_path_buf()
    } else {
        return;
    };

    if dir.exists() {
        if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
            tracing::warn!("could not watch {:?}: {e}", dir);
        }
    }
}

/// Guard that keeps the watcher alive for as long as it is held.
pub struct ReloadGuard {
    _watcher: RecommendedWatcher,
}
