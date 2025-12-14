use std::path::{PathBuf, Path};
use notify::{recommended_watcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::sync::mpsc;
use glob::{Pattern, MatchOptions};

use crate::FileChangeType;
use std::sync::Arc;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::Mutex;

pub type FileChangeCallback =
    Arc<Mutex<dyn Fn(&str, FileChangeType) -> crate::Result<()> + Send + Sync>>;

#[derive(Clone)]
pub enum IgnorePattern {
    Glob(Pattern),
    Exact(String),
}

pub struct FileWatcher {
    file_change_callback: FileChangeCallback,
    watched_paths: HashMap<PathBuf, bool>,
    ignore_patterns: Vec<IgnorePattern>,
    // Keep a copy of raw patterns if needed, but we can also just expose the behavior.
    // For `get_ignore_patterns`, we will need to reconstruct strings or return this list.
    // To match existing API, let's keep the raw strings separate or derive them.
    // Given the previous code just returned `&Vec<String>`, let's store `raw_ignore_patterns` too.
    raw_ignore_patterns: Vec<String>,
}

// Helper to determine if we should use Glob or Exact
fn parse_ignore_pattern(pattern: &str) -> IgnorePattern {
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        match Pattern::new(pattern) {
            Ok(p) => IgnorePattern::Glob(p),
            Err(e) => {
                eprintln!("Warning: Invalid glob pattern '{}': {}. Falling back to exact match.", pattern, e);
                IgnorePattern::Exact(pattern.to_string())
            }
        }
    } else {
        IgnorePattern::Exact(pattern.to_string())
    }
}

// Static helper to avoid code duplication and allow usage without &self (e.g. in threads)
fn should_ignore_path(path: &Path, ignore_patterns: &[IgnorePattern]) -> bool {
    let path_str = path.to_string_lossy();
    // Normalize path separators to forward slashes for glob matching (Windows compatibility)
    let normalized_path = if std::path::MAIN_SEPARATOR == '\\' {
        path_str.replace('\\', "/")
    } else {
        path_str.into_owned()
    };

    let match_options = MatchOptions {
        case_sensitive: true, // Default
        require_literal_separator: true, // Do not allow * to match /
        require_literal_leading_dot: false, // Default
    };

    ignore_patterns.iter().any(|pattern| {
        match pattern {
            IgnorePattern::Glob(p) => p.matches_with(&normalized_path, match_options),
            IgnorePattern::Exact(p) => {
                 // Exact path component matching
                // Check if the pattern matches a path component or the end of the path
                // This prevents "target" from matching "src/targets/file.rs"
                for component in path.components() {
                    if let Some(comp_str) = component.as_os_str().to_str() {
                        if comp_str == p {
                            return true;
                        }
                    }
                }
                false
            }
        }
    })
}

impl FileWatcher {
    pub fn new(file_change_callback: FileChangeCallback) -> crate::Result<Self> {
        let defaults: &[&str] = &[
            ".git",
            "target",
            "node_modules",
            ".DS_Store",
            "**/*.tmp",
            "**/*.log",
        ];
        let ignore_patterns = defaults.iter().map(|s| parse_ignore_pattern(s)).collect();
        Ok(Self {
            file_change_callback,
            watched_paths: HashMap::new(),
            ignore_patterns,
            raw_ignore_patterns: defaults.iter().map(|s| s.to_string()).collect(),
        })
    }

    pub fn add_watch_path(&mut self, path: PathBuf, recursive: bool) -> crate::Result<()> {
        self.watched_paths.insert(path.clone(), recursive);
        Ok(())
    }

    pub fn remove_watch_path(&mut self, path: &PathBuf) -> crate::Result<()> {
        self.watched_paths.remove(path);
        Ok(())
    }

    pub fn add_ignore_pattern(&mut self, pattern: String) {
        self.ignore_patterns.push(parse_ignore_pattern(&pattern));
        self.raw_ignore_patterns.push(pattern);
    }

    pub fn should_ignore(&self, path: &Path) -> bool {
        should_ignore_path(path, &self.ignore_patterns)
    }

    pub async fn start_watching(&mut self) -> crate::Result<()> {
        let (tx, mut rx) = tokio_mpsc::channel(100);

        // Spawn the file watcher in a separate thread
        let watched_paths = self.watched_paths.clone();
        let ignore_patterns = self.ignore_patterns.clone();

        std::thread::spawn(move || {
            let (notify_tx, notify_rx) = mpsc::channel();

            let mut watcher = recommended_watcher(notify_tx).unwrap();

            // Watch all registered paths
            for (path, recursive) in &watched_paths {
                let mode = if *recursive {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                if let Err(e) = watcher.watch(path, mode) {
                    eprintln!("Failed to watch path {:?}: {}", path, e);
                }
            }

            // Process file system events
            loop {
                match notify_rx.recv() {
                    Ok(Ok(event)) => {
                        // Filter out ignored files
                        let notify::Event { paths, .. } = &event;
                        
                        let should_process = paths.iter().all(|path| {
                            !should_ignore_path(path, &ignore_patterns)
                        });
                        
                        if should_process {
                            if let Err(e) = tx.blocking_send(event) {
                                eprintln!("Failed to send file event: {}", e);
                                break;
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        eprintln!("File watcher error: {}", e);
                    }
                    Err(e) => {
                        eprintln!("File watcher channel error: {}", e);
                        break;
                    }
                }
            }
        });

        // Process events in the async context
        while let Some(event) = rx.recv().await {
            self.process_file_event(event).await?;
        }

        Ok(())
    }

    async fn process_file_event(&mut self, event: notify::Event) -> crate::Result<()> {
        if let notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) = event.kind {
            // Handle rename: expect 2 paths [old, new]
            if event.paths.len() == 2 {
                let old_path = event.paths[0].to_string_lossy().to_string();
                let new_path = event.paths[1].to_string_lossy().to_string();
                let change = FileChangeType::Renamed { old_path };
                let callback = self.file_change_callback.clone();
                tokio::spawn(async move {
                    if let Err(e) = callback.lock().await(&new_path, change) {
                        eprintln!("Error in file change callback: {}", e);
                    }
                });
                return Ok(());
            }
        }

        for path in &event.paths {
            let change_type = match event.kind {
                notify::EventKind::Create(_) => FileChangeType::Created,
                notify::EventKind::Remove(_) => FileChangeType::Deleted,
                notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                    // For rename events, if not 2 paths, we can't do much or treat as modify/create
                    // Fallback if we only got 1 path for some reason (rare for Rename)
                    eprintln!("Warning: Received rename event with {} paths, expected 2. Fallback to rename with empty old_path.", event.paths.len());
                    FileChangeType::Renamed { old_path: String::new() }
                }
                notify::EventKind::Modify(_) => FileChangeType::Modified,
                _ => continue, // Skip other event types
            };

            let callback = self.file_change_callback.clone();
            let path_str = path.to_string_lossy().to_string();

            let event_kind = event.kind.clone();
            tokio::spawn(async move {
                let change = match event_kind {
                    notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                        // For rename, try to read old path from first element if present
                        // Since we're inside spawned task, we only have current path; this is a best-effort placeholder
                        if let FileChangeType::Renamed { .. } = &change_type {
                            FileChangeType::Renamed {
                                old_path: String::from(""),
                            }
                        } else {
                            change_type
                        }
                    }
                    _ => change_type,
                };
                if let Err(e) = callback.lock().await(&path_str, change) {
                    eprintln!("Error in file change callback: {}", e);
                }
            });
        }

        Ok(())
    }

    pub fn get_watched_paths(&self) -> &HashMap<PathBuf, bool> {
        &self.watched_paths
    }

    pub fn get_ignore_patterns(&self) -> &Vec<String> {
        &self.raw_ignore_patterns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventRecorder;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    #[test]
    fn test_ignore_patterns() {
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("events.db");
        let storage = crate::storage::Storage::with_path(db_path.to_str().unwrap()).unwrap();
        let _event_recorder = Arc::new(Mutex::new(EventRecorder::with_storage(
            "test-session",
            storage,
        )));

        // Create a callback function that matches the expected signature
        let callback: FileChangeCallback = Arc::new(Mutex::new(
            move |_path: &str, _change_type: FileChangeType| Ok(()),
        ));
        let mut file_watcher = FileWatcher::new(callback).unwrap();

        // Test basic ignore patterns
        assert!(file_watcher.should_ignore(&PathBuf::from(".git/config")));
        assert!(file_watcher.should_ignore(&PathBuf::from("target/debug/app")));
        assert!(file_watcher.should_ignore(&PathBuf::from("node_modules/lodash")));
        // Test default globs
        assert!(file_watcher.should_ignore(&PathBuf::from("temp.tmp")));
        assert!(file_watcher.should_ignore(&PathBuf::from("src/temp.tmp"))); // Recursive due to **
        assert!(!file_watcher.should_ignore(&PathBuf::from("temp.txt")));

        // Test glob patterns added manually
        file_watcher.add_ignore_pattern("*.custom".to_string()); // Non recursive because required literal separator
        assert!(file_watcher.should_ignore(&PathBuf::from("file.custom")));
        assert!(!file_watcher.should_ignore(&PathBuf::from("src/file.custom"))); // Should NOT match now

        file_watcher.add_ignore_pattern("**/*.recursive".to_string());
        assert!(file_watcher.should_ignore(&PathBuf::from("src/test.recursive")));

        // Test more complex glob patterns supported by glob crate
        file_watcher.add_ignore_pattern("src/**/*.rs.bk".to_string());
        assert!(file_watcher.should_ignore(&PathBuf::from("src/utils/file.rs.bk")));
        assert!(!file_watcher.should_ignore(&PathBuf::from("src/utils/file.rs")));
    }
}
