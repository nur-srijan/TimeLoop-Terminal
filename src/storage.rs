use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::sync::{RwLock, Arc};
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use crate::Event;
use crate::session::Session;
use crate::branch::TimelineBranch;

#[derive(Default, Clone, Serialize, Deserialize)]
struct StorageInner {
    events: HashMap<String, Vec<Event>>,      // session_id -> events
    sessions: HashMap<String, Session>,       // session_id -> session
    branches: HashMap<String, TimelineBranch>,// branch_id -> branch
}

static GLOBAL_STORAGE: Lazy<RwLock<StorageInner>> = Lazy::new(|| RwLock::new(StorageInner::default()));

pub struct Storage {
    // When `inner` is None, operations go to the global singleton (and persist to the global location).
    // When `inner` is Some(...), this Storage instance operates on an independent in-memory store and
    // may optionally persist to the specified `persistence_path`.
    inner: Option<Arc<RwLock<StorageInner>>>,
    persistence_path: Option<PathBuf>,
}

impl Storage {
    pub fn new() -> crate::Result<Self> {
        // Best-effort load persisted state for the global storage
        let _ = Self::load_from_disk();
        Ok(Self { inner: None, persistence_path: None })
    }

    // `with_path` creates an isolated storage instance whose state is stored in the
    // provided path. If the path exists it will be loaded into memory; mutations on
    // the Storage instance will be persisted to that path. This is useful for
    // integration tests that need on-disk isolation.
    pub fn with_path(path: &str) -> crate::Result<Self> {
        let pb = PathBuf::from(path);
        let inner = Arc::new(RwLock::new(StorageInner::default()));
        let storage = Self { inner: Some(inner.clone()), persistence_path: Some(pb.clone()) };

        // If the file exists, load it into the per-instance inner store
        if pb.exists() {
            if let Ok(data) = std::fs::read_to_string(&pb) {
                if let Ok(inner_data) = serde_json::from_str::<StorageInner>(&data) {
                    if let Ok(mut guard) = inner.write() {
                        *guard = inner_data;
                    }
                }
            }
        }

        Ok(storage)
    }

    pub fn get_db_path() -> crate::Result<std::path::PathBuf> {
        Ok(std::path::PathBuf::from("/tmp/timeloop-memory"))
    }

    // Helper to run read-only closures against the correct storage instance
    fn with_read<F, R>(&self, f: F) -> crate::Result<R>
    where
        F: FnOnce(&StorageInner) -> R,
    {
        if let Some(inner) = &self.inner {
            let guard = inner.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&*guard))
        } else {
            let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&*guard))
        }
    }

    // Helper to run write closures against the correct storage instance
    fn with_write<F, R>(&self, f: F) -> crate::Result<R>
    where
        F: FnOnce(&mut StorageInner) -> R,
    {
        if let Some(inner) = &self.inner {
            let mut guard = inner.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&mut *guard))
        } else {
            let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
            Ok(f(&mut *guard))
        }
    }

    pub fn store_event(&self, event: &Event) -> crate::Result<()> {
        self.with_write(|guard| {
            let session_events = guard.events.entry(event.session_id.clone()).or_insert_with(Vec::new);
            session_events.push(event.clone());
        })?;
        // Persist if global storage is used, or if this instance has a persistence path
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> crate::Result<Vec<Event>> {
        self.with_read(|guard| guard.events.get(session_id).cloned().unwrap_or_default())
    }

    pub fn get_events_in_range(&self, session_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> crate::Result<Vec<Event>> {
        let events = self.get_events_for_session(session_id)?;
        let filtered: Vec<Event> = events.into_iter().filter(|e| e.timestamp >= start && e.timestamp <= end).collect();
        Ok(filtered)
    }

    pub fn get_last_event(&self, session_id: &str) -> crate::Result<Option<Event>> {
        let events = self.get_events_for_session(session_id)?;
        Ok(events.last().cloned())
    }

    pub fn clear_session_events(&self, session_id: &str) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.events.remove(session_id);
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    // Session management
    pub fn store_session(&self, session: &Session) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.sessions.insert(session.id.clone(), session.clone());
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> crate::Result<Option<Session>> {
        self.with_read(|guard| guard.sessions.get(session_id).cloned())
    }

    pub fn list_sessions(&self) -> crate::Result<Vec<Session>> {
        self.with_read(|guard| {
            let mut sessions: Vec<Session> = guard.sessions.values().cloned().collect();
            sessions.sort_by_key(|s| s.created_at);
            sessions
        })
    }

    // Branch management
    pub fn store_branch(&self, branch: &TimelineBranch) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.branches.insert(branch.id.clone(), branch.clone());
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn get_branch(&self, branch_id: &str) -> crate::Result<Option<TimelineBranch>> {
        self.with_read(|guard| guard.branches.get(branch_id).cloned())
    }

    pub fn list_branches(&self) -> crate::Result<Vec<TimelineBranch>> {
        self.with_read(|guard| {
            let mut branches: Vec<TimelineBranch> = guard.branches.values().cloned().collect();
            branches.sort_by_key(|b| b.created_at);
            branches
        })
    }

    pub fn delete_session(&self, session_id: &str) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.events.remove(session_id);
            guard.sessions.remove(session_id);
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    pub fn delete_branch(&self, branch_id: &str) -> crate::Result<()> {
        self.with_write(|guard| {
            guard.events.remove(branch_id);
            guard.branches.remove(branch_id);
        })?;
        if let Some(path) = &self.persistence_path {
            let _ = Self::save_to_path(path, self);
        } else if self.inner.is_none() {
            let _ = Self::save_to_disk();
        }
        Ok(())
    }

    // Simple JSON export/import for sessions
    pub fn export_session_to_file(&self, session_id: &str, path: &str) -> crate::Result<()> {
        let session = self.get_session(session_id)?.ok_or_else(|| crate::error::TimeLoopError::SessionNotFound(session_id.to_string()))?;
        let events = self.get_events_for_session(session_id)?;
        let bundle = SessionExport { session, events };
        let json = serde_json::to_string_pretty(&bundle)?;
        let mut file = fs::File::create(path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        file.write_all(json.as_bytes()).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        Ok(())
    }

    pub fn import_session_from_file(&self, path: &str) -> crate::Result<String> {
        let data = fs::read_to_string(path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let bundle: SessionExport = serde_json::from_str(&data)?;
        let id = bundle.session.id.clone();
        self.store_session(&bundle.session)?;
        for event in &bundle.events {
            self.store_event(event)?;
        }
        Ok(id)
    }
}

#[derive(Serialize, Deserialize)]
struct SessionExport {
    session: Session,
    events: Vec<Event>,
}

impl Storage {
    fn data_dir() -> std::path::PathBuf {
        if cfg!(target_os = "windows") {
            std::env::var("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("timeloop-terminal")
        } else if cfg!(target_os = "macos") {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("Library/Application Support/timeloop-terminal")
        } else {
            // Linux and others
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".local/share/timeloop-terminal")
        }
    }

    fn persistence_file() -> std::path::PathBuf {
        Self::data_dir().join("state.json")
    }

    fn load_from_disk() -> crate::Result<()> {
        let path = Self::persistence_file();
        if !path.exists() {
            return Ok(());
        }
        let data = fs::read_to_string(&path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let inner: StorageInner = serde_json::from_str(&data)?;
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        *guard = inner;
        Ok(())
    }

    // Save to a per-instance path. Serialize the current inner state (either global
    // or the instance's inner) and write it to the provided path.
    fn save_to_path(path: &PathBuf, storage: &Storage) -> crate::Result<()> {
        // Determine which inner to read from
        let data_inner = if let Some(inner) = &storage.inner {
            inner.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?.clone()
        } else {
            GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?.clone()
        };

        // Ensure parent dir exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        }

        let data = serde_json::to_string_pretty(&data_inner)?;
        // Perform an atomic write: write to a temp file and rename into place. This
        // reduces the chance of file corruption if the process is interrupted while
        // writing. Note: on some platforms rename over an existing file may not be
        // atomic across filesystems; avoid sharing the same file across mounts.
        Self::atomic_write(path, data.as_bytes())?;
        Ok(())
    }

    /// Public API: explicitly flush the storage state to disk. If this Storage
    /// instance has an associated persistence path it will be saved there; otherwise
    /// the global storage will be saved to the default location.
    pub fn flush(&self) -> crate::Result<()> {
        if let Some(path) = &self.persistence_path {
            Self::save_to_path(path, self)
        } else {
            Self::save_to_disk()
        }
    }

    // Helper to atomically write bytes to a file path. Writes to a temporary file in
    // the same directory and then renames into place.
    fn atomic_write(path: &PathBuf, bytes: &[u8]) -> crate::Result<()> {
        let parent = path.parent().ok_or_else(|| crate::error::TimeLoopError::FileSystem("Invalid path".to_string()))?;
        let mut tmp = parent.join(".tmp_timeloop");
        // add a random suffix to avoid collisions
        use rand::{thread_rng, Rng};
        let suffix: u64 = thread_rng().gen();
        tmp = tmp.with_extension(format!("{}.tmp", suffix));

        // Write tmp file
        fs::write(&tmp, bytes).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        // Rename into place (atomic on most platforms when on same filesystem)
        std::fs::rename(&tmp, path).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        Ok(())
    }

    fn save_to_disk() -> crate::Result<()> {
        let dir = Self::data_dir();
        fs::create_dir_all(&dir).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        let path = Self::persistence_file();
        let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        let data = serde_json::to_string_pretty(&*guard)?;
        fs::write(&path, data).map_err(|e| crate::error::TimeLoopError::FileSystem(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventType;
    use uuid::Uuid;
    use tempfile::TempDir;

    #[test]
    fn test_in_memory_storage() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");
        let storage = Storage::with_path(state_file.to_str().unwrap()).unwrap();
        
        // Test session storage
        let session = Session {
            id: "test-session".to_string(),
            name: "Test Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        
        storage.store_session(&session).unwrap();
        let retrieved = storage.get_session("test-session").unwrap().unwrap();
        assert_eq!(retrieved.id, "test-session");
        
        // Test event storage
        let event = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "test-session".to_string(),
            event_type: EventType::KeyPress {
                key: "a".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };
        
        storage.store_event(&event).unwrap();
        let events = storage.get_events_for_session("test-session").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");
        let storage = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        // Initial state: empty
        assert!(storage.list_sessions().unwrap().is_empty());
        assert!(storage.list_branches().unwrap().is_empty());

        // Create session and branch
        let session = Session {
            id: "roundtrip-session".to_string(),
            name: "Roundtrip Session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage.store_session(&session).unwrap();

        let branch = TimelineBranch {
            id: "roundtrip-branch".to_string(),
            name: "Roundtrip Branch".to_string(),
            parent_session_id: "roundtrip-session".to_string(),
            branch_point_event_id: "".to_string(),
            created_at: Utc::now(),
            description: None,
        };
        storage.store_branch(&branch).unwrap();

        // Verify stored state
        let sessions = storage.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "roundtrip-session");

        let branches = storage.list_branches().unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].id, "roundtrip-branch");

        // Write some events
        let event1 = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "roundtrip-session".to_string(),
            event_type: EventType::KeyPress {
                key: "a".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 1,
            timestamp: Utc::now(),
        };
        storage.store_event(&event1).unwrap();

        let event2 = Event {
            id: Uuid::new_v4().to_string(),
            session_id: "roundtrip-session".to_string(),
            event_type: EventType::KeyPress {
                key: "b".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: 2,
            timestamp: Utc::now(),
        };
        storage.store_event(&event2).unwrap();

        // Flush to persist
        storage.flush().unwrap();

        // Drop storage to close file handles
        drop(storage);

        // Reopen storage
        let storage = Storage::with_path("test-persistence-roundtrip").unwrap();

        // Verify restored state
        let sessions = storage.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "roundtrip-session");

        let branches = storage.list_branches().unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].id, "roundtrip-branch");

        // Verify events
        let events = storage.get_events_for_session("roundtrip-session").unwrap();
        assert_eq!(events.len(), 2);
        // Compare key values to avoid asserting on timestamps directly
        if let EventType::KeyPress { key, .. } = &events[0].event_type {
            assert_eq!(key, "a");
        } else { panic!("expected key press event"); }
        if let EventType::KeyPress { key, .. } = &events[1].event_type {
            assert_eq!(key, "b");
        } else { panic!("expected key press event"); }
    }

    #[test]
    fn test_persistence_roundtrip_tempdir() {
        let tmp_dir = TempDir::new().unwrap();
        let state_file = tmp_dir.path().join("state.json");

        // Create first storage instance backed by the file
        let storage1 = Storage::with_path(state_file.to_str().unwrap()).unwrap();

        let session = Session {
            id: "persistence-test".to_string(),
            name: "Persistence Test".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };
        storage1.store_session(&session).unwrap();
        storage1.flush().unwrap();

        // Create a second storage instance pointing at the same file and verify
        // data persisted.
        let storage2 = Storage::with_path(state_file.to_str().unwrap()).unwrap();
        let retrieved = storage2.get_session("persistence-test").unwrap().unwrap();
        assert_eq!(retrieved.id, "persistence-test");
    }
}