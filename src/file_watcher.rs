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

pub struct FileWatcher {
    file_change_callback: FileChangeCallback,
    watched_paths: HashMap<PathBuf, bool>,
    ignore_patterns: Vec<String>,
}

impl FileWatcher {
    pub fn new(file_change_callback: FileChangeCallback) -> crate::Result<Self> {
        Ok(Self {
            file_change_callback,
            watched_paths: HashMap::new(),
            ignore_patterns: vec![
                ".git".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                ".DS_Store".to_string(),
                "*.tmp".to_string(),
                "*.log".to_string(),
            ],
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
        self.ignore_patterns.push(pattern);
    }

    pub fn should_ignore(&self, path: &PathBuf) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.ignore_patterns {
            if pattern.contains('*') {
                // Simple glob pattern matching
                if self.matches_glob(&path_str, pattern) {
                    return true;
                }
            } else if path_str.contains(pattern) {
                return true;
            }
        }

        false
    }

    fn matches_glob(&self, path: &str, pattern: &str) -> bool {
        // Simple glob matching - can be enhanced with a proper glob library
        if pattern == "*" {
            return true;
        }

        if pattern.starts_with("*.") {
            let ext = &pattern[1..];
            return path.ends_with(ext);
        }

        if pattern.ends_with("*") {
            let prefix = &pattern[..pattern.len() - 1];
            return path.starts_with(prefix);
        }

        path == pattern
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
                        let should_process = match &event {
                            notify::Event { paths, .. } => {
                                paths.iter().all(|path| {
                                    // Optimization: Calculate path string once for all pattern checks
                                    // and avoid cloning the ignore_patterns vector
                                    let path_str = path.to_string_lossy();

                                    !ignore_patterns.iter().any(|pattern| {
                                        if pattern.contains('*') {
                                            // Simple glob matching
                                            if pattern == "*" {
                                                return true;
                                            }
                                            if pattern.starts_with("*.") {
                                                let ext = &pattern[1..];
                                                return path_str.ends_with(ext);
                                            }
                                            if pattern.ends_with("*") {
                                                let prefix = &pattern[..pattern.len() - 1];
                                                return path_str.starts_with(prefix);
                                            }
                                            false
                                        } else {
                                            path_str.contains(pattern)
                                        }
                                    })
                                })
                            }
                        };

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
