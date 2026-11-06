//! Watches the profiles folder; any `*.toml` change triggers `on_change` (debounced by the app).

use std::path::Path;

use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::is_profile_file;

/// Keep the returned watcher alive for as long as changes should be reported.
pub fn watch(dir: &Path, on_change: impl Fn() + Send + 'static) -> Result<RecommendedWatcher> {
    let mut watcher =
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
            Ok(event)
                if !matches!(event.kind, EventKind::Access(_))
                    && event.paths.iter().any(|p| is_profile_file(p)) =>
            {
                on_change()
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "profile watcher error"),
        })
        .context("create profile watcher")?;
    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("watch {dir:?}"))?;
    Ok(watcher)
}
