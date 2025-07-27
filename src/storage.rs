use sled;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::time::Duration;
use std::thread;
use crate::{Event, TimeLoopError};
use tracing::{warn, info};

pub struct Storage {
    pub(crate) db: sled::Db,
}

impl Storage {
    pub fn new() -> crate::Result<Self> {
        let db_path = Self::get_db_path()?;
        let db = Self::open_with_retry(&db_path.to_string_lossy())?;
        Ok(Self { db })
    }

    pub fn with_path(path: &str) -> crate::Result<Self> {
        let db = Self::open_with_retry(path)?;
        Ok(Self { db })
    }

    /// Opens sled database with retry logic and exponential backoff
    fn open_with_retry(path: &str) -> crate::Result<sled::Db> {
        const MAX_RETRIES: u32 = 5;
        const INITIAL_DELAY_MS: u64 = 100;
        
        for attempt in 0..MAX_RETRIES {
            match sled::open(path) {
                Ok(db) => {
                    if attempt > 0 {
                        info!("Successfully opened database after {} retries", attempt);
                    }
                    return Ok(db);
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    
                    // Check if this is a lock-related error
                    if Self::is_lock_error(&error_msg) {
                        if attempt < MAX_RETRIES - 1 {
                            let delay = INITIAL_DELAY_MS * 2_u64.pow(attempt);
                            warn!(
                                "Database locked (attempt {}/{}), retrying in {}ms: {}", 
                                attempt + 1, MAX_RETRIES, delay, error_msg
                            );
                            thread::sleep(Duration::from_millis(delay));
                            continue;
                        } else {
                            return Err(TimeLoopError::Database(format!(
                                "Failed to acquire database lock after {} attempts. \
                                Another instance of TimeLoop Terminal may be running. \
                                Please close other instances or wait for them to finish.\n\
                                Original error: {}", 
                                MAX_RETRIES, error_msg
                            )));
                        }
                    } else {
                        // Non-lock error, fail immediately
                        return Err(TimeLoopError::Database(format!(
                            "Database error: {}", error_msg
                        )));
                    }
                }
            }
        }
        
        unreachable!("Should have returned or failed by now");
    }

    /// Check if error message indicates a lock contention issue
    fn is_lock_error(error_msg: &str) -> bool {
        let error_lower = error_msg.to_lowercase();
        error_lower.contains("lock") || 
        error_lower.contains("resource temporarily unavailable") ||
        error_lower.contains("would block") ||
        error_lower.contains("another process has locked") ||
        error_lower.contains("database is locked")
    }

    pub fn get_db_path() -> crate::Result<PathBuf> {
        let mut path = dirs::data_local_dir()
            .ok_or_else(|| TimeLoopError::Configuration("Could not find local data directory".to_string()))?;
        path.push("timeloop-terminal");
        std::fs::create_dir_all(&path)?;
        path.push("events.db");
        Ok(path)
    }

    pub fn store_event(&self, event: &Event) -> crate::Result<()> {
        let key = format!("event:{}:{}", event.session_id, event.sequence_number);
        let value = serde_json::to_vec(event)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> crate::Result<Vec<Event>> {
        let mut events = Vec::new();
        let prefix = format!("event:{}:", session_id);
        
        for result in self.db.scan_prefix(prefix) {
            let (_, value) = result?;
            let event: Event = serde_json::from_slice(&value)?;
            events.push(event);
        }
        
        events.sort_by_key(|e| e.sequence_number);
        Ok(events)
    }

    pub fn get_events_in_range(&self, session_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> crate::Result<Vec<Event>> {
        let events = self.get_events_for_session(session_id)?;
        let filtered: Vec<Event> = events
            .into_iter()
            .filter(|e| e.timestamp >= start && e.timestamp <= end)
            .collect();
        Ok(filtered)
    }

    pub fn get_last_event(&self, session_id: &str) -> crate::Result<Option<Event>> {
        let events = self.get_events_for_session(session_id)?;
        Ok(events.last().cloned())
    }

    pub fn clear_session_events(&self, session_id: &str) -> crate::Result<()> {
        let prefix = format!("event:{}:", session_id);
        let keys: Vec<_> = self.db.scan_prefix(&prefix).collect();
        
        for result in keys {
            let (key, _) = result?;
            self.db.remove(key)?;
        }
        
        Ok(())
    }

    // Session management
    pub fn store_session(&self, session: &Session) -> crate::Result<()> {
        let key = format!("session:{}", session.id);
        let value = serde_json::to_vec(session)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> crate::Result<Option<Session>> {
        let key = format!("session:{}", session_id);
        if let Some(value) = self.db.get(key)? {
            let session: Session = serde_json::from_slice(&value)?;
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    pub fn list_sessions(&self) -> crate::Result<Vec<Session>> {
        let mut sessions = Vec::new();
        
        for result in self.db.scan_prefix("session:") {
            let (_, value) = result?;
            let session: Session = serde_json::from_slice(&value)?;
            sessions.push(session);
        }
        
        sessions.sort_by_key(|s| s.created_at);
        Ok(sessions)
    }

    // Branch management
    pub fn store_branch(&self, branch: &TimelineBranch) -> crate::Result<()> {
        let key = format!("branch:{}", branch.id);
        let value = serde_json::to_vec(branch)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    pub fn get_branch(&self, branch_id: &str) -> crate::Result<Option<TimelineBranch>> {
        let key = format!("branch:{}", branch_id);
        if let Some(value) = self.db.get(key)? {
            let branch: TimelineBranch = serde_json::from_slice(&value)?;
            Ok(Some(branch))
        } else {
            Ok(None)
        }
    }

    pub fn list_branches(&self) -> crate::Result<Vec<TimelineBranch>> {
        let mut branches = Vec::new();
        
        for result in self.db.scan_prefix("branch:") {
            let (_, value) = result?;
            let branch: TimelineBranch = serde_json::from_slice(&value)?;
            branches.push(branch);
        }
        
        branches.sort_by_key(|b| b.created_at);
        Ok(branches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use tempfile::TempDir;

    #[test]
    fn test_concurrent_database_access() {
        // Create a temporary directory for the test database
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test_concurrent.db");
        let db_path_str = db_path.to_str().unwrap();

        // First, create and close a database to ensure it exists
        {
            let _storage = Storage::with_path(db_path_str).expect("Failed to create initial database");
        }

        // Now test concurrent access
        let path = Arc::new(db_path_str.to_string());
        let mut handles = vec![];

        // Spawn multiple threads trying to access the same database
        for i in 0..3 {
            let path_clone = Arc::clone(&path);
            let handle = thread::spawn(move || {
                println!("Thread {} attempting to open database", i);
                match Storage::with_path(&path_clone) {
                    Ok(_storage) => {
                        println!("Thread {} successfully opened database", i);
                        thread::sleep(std::time::Duration::from_millis(100)); // Hold the database briefly
                        true
                    }
                    Err(e) => {
                        println!("Thread {} failed to open database: {}", i, e);
                        false
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        let results: Vec<bool> = handles.into_iter()
            .map(|h| h.join().unwrap())
            .collect();

        // At least one thread should succeed
        assert!(results.iter().any(|&success| success), 
                "At least one thread should successfully open the database");
        
        println!("Test completed. Results: {:?}", results);
    }

    #[test]
    fn test_lock_error_detection() {
        // Test various error messages that should be detected as lock errors
        let lock_errors = vec![
            "Resource temporarily unavailable",
            "database is locked",
            "another process has locked a portion of the file",
            "WouldBlock",
            "LOCK error",
        ];

        for error_msg in lock_errors {
            assert!(Storage::is_lock_error(error_msg), 
                    "Should detect '{}' as a lock error", error_msg);
        }

        // Test non-lock errors
        let non_lock_errors = vec![
            "Permission denied",
            "File not found",
            "Invalid format",
        ];

        for error_msg in non_lock_errors {
            assert!(!Storage::is_lock_error(error_msg), 
                    "Should not detect '{}' as a lock error", error_msg);
        }
    }
}

// Re-export types that are used in this module
use crate::session::Session;
use crate::branch::TimelineBranch; 