use notify::{recommended_watcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::FileChangeType;
use std::sync::Arc;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::Mutex;

pub type FileChangeCallback =
    Arc<Mutex<dyn Fn(&str, FileChangeType) -> crate::Result<()> + Send + Sync>>;

#[derive(Clone, Debug)]
struct PatternMatcher {
    extensions: Vec<String>,
    prefixes: Vec<String>,
    substrings: Vec<String>,
    exact: Vec<String>,
    match_all: bool,
}

impl PatternMatcher {
    fn new(patterns: &[String]) -> Self {
        let mut matcher = Self {
            extensions: Vec::new(),
            prefixes: Vec::new(),
            substrings: Vec::new(),
            exact: Vec::new(),
            match_all: false,
        };
        for p in patterns {
            matcher.add_pattern(p);
        }
        matcher
    }

    fn add_pattern(&mut self, pattern: &str) {
        if pattern == "*" {
            self.match_all = true;
            return;
        }

        if pattern.contains('*') {
            if pattern.starts_with("*.") {
                self.extensions.push(pattern[1..].to_string());
            } else if pattern.ends_with("*") {
                self.prefixes.push(pattern[..pattern.len() - 1].to_string());
            } else {
                self.exact.push(pattern.to_string());
            }
        } else {
            self.substrings.push(pattern.to_string());
        }
    }

    fn matches(&self, path: &str) -> bool {
        if self.match_all {
            return true;
        }

        for ext in &self.extensions {
            if path.ends_with(ext) {
                return true;
            }
        }

        for pre in &self.prefixes {
            if path.starts_with(pre) {
                return true;
            }
        }

        for sub in &self.substrings {
            if path.contains(sub) {
                return true;
            }
        }

        for ex in &self.exact {
            if path == ex {
                return true;
            }
        }

        false
    }
}

pub struct FileWatcher {
    file_change_callback: FileChangeCallback,
    watched_paths: HashMap<PathBuf, bool>,
    ignore_patterns: Vec<String>,
    matcher: PatternMatcher,
}

impl FileWatcher {
    pub fn new(file_change_callback: FileChangeCallback) -> crate::Result<Self> {
        let ignore_patterns = vec![
            ".git".to_string(),
            "target".to_string(),
            "node_modules".to_string(),
            ".DS_Store".to_string(),
            "*.tmp".to_string(),
            "*.log".to_string(),
        ];
        let matcher = PatternMatcher::new(&ignore_patterns);

        Ok(Self {
            file_change_callback,
            watched_paths: HashMap::new(),
            ignore_patterns,
            matcher,
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
        self.matcher.add_pattern(&pattern);
        self.ignore_patterns.push(pattern);
    }

    pub fn should_ignore(&self, path: &std::path::Path) -> bool {
        self.matcher.matches(&path.to_string_lossy())
    }

    pub async fn start_watching(&mut self) -> crate::Result<()> {
        let (tx, mut rx) = tokio_mpsc::channel(100);

        // Spawn the file watcher in a separate thread
        let watched_paths = self.watched_paths.clone();
        let matcher = self.matcher.clone();

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
                            // Optimization: Calculate path string once for all pattern checks
                            let path_str = path.to_string_lossy();
                            !matcher.matches(&path_str)
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
        for path in event.paths {
            let change_type = match event.kind {
                notify::EventKind::Create(_) => FileChangeType::Created,
                notify::EventKind::Remove(_) => FileChangeType::Deleted,
                notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                    // For rename events, capture rename change
                    FileChangeType::Renamed {
                        old_path: String::new(),
                    }
                }
                notify::EventKind::Modify(_) => FileChangeType::Modified,
                _ => continue, // Skip other event types
            };

            let callback = self.file_change_callback.clone();
            let path_str = path.to_string_lossy().to_string();
            let event_kind = event.kind;
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
        &self.ignore_patterns
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
        assert!(file_watcher.should_ignore(&PathBuf::from("app.log")));

        // Test glob patterns
        file_watcher.add_ignore_pattern("*.tmp".to_string());
        assert!(file_watcher.should_ignore(&PathBuf::from("temp.tmp")));
        assert!(!file_watcher.should_ignore(&PathBuf::from("temp.txt")));
    }
}
