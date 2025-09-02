use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::sync::RwLock;
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

pub struct Storage;

impl Storage {
    pub fn new() -> crate::Result<Self> {
        // Best-effort load persisted state
        let _ = Self::load_from_disk();
        Ok(Self)
    }

    pub fn with_path(_path: &str) -> crate::Result<Self> { Self::new() }

    pub fn get_db_path() -> crate::Result<std::path::PathBuf> {
        Ok(std::path::PathBuf::from("/tmp/timeloop-memory"))
    }

    pub fn store_event(&self, event: &Event) -> crate::Result<()> {
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        let session_events = guard.events.entry(event.session_id.clone()).or_insert_with(Vec::new);
        session_events.push(event.clone());
        drop(guard);
        let _ = Self::save_to_disk();
        Ok(())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> crate::Result<Vec<Event>> {
        let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        Ok(guard.events.get(session_id).cloned().unwrap_or_default())
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
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        guard.events.remove(session_id);
        drop(guard);
        let _ = Self::save_to_disk();
        Ok(())
    }

    // Session management
    pub fn store_session(&self, session: &Session) -> crate::Result<()> {
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        guard.sessions.insert(session.id.clone(), session.clone());
        drop(guard);
        let _ = Self::save_to_disk();
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> crate::Result<Option<Session>> {
        let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        Ok(guard.sessions.get(session_id).cloned())
    }

    pub fn list_sessions(&self) -> crate::Result<Vec<Session>> {
        let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        let mut sessions: Vec<Session> = guard.sessions.values().cloned().collect();
        sessions.sort_by_key(|s| s.created_at);
        Ok(sessions)
    }

    // Branch management
    pub fn store_branch(&self, branch: &TimelineBranch) -> crate::Result<()> {
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        guard.branches.insert(branch.id.clone(), branch.clone());
        drop(guard);
        let _ = Self::save_to_disk();
        Ok(())
    }

    pub fn get_branch(&self, branch_id: &str) -> crate::Result<Option<TimelineBranch>> {
        let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        Ok(guard.branches.get(branch_id).cloned())
    }

    pub fn list_branches(&self) -> crate::Result<Vec<TimelineBranch>> {
        let guard = GLOBAL_STORAGE.read().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        let mut branches: Vec<TimelineBranch> = guard.branches.values().cloned().collect();
        branches.sort_by_key(|b| b.created_at);
        Ok(branches)
    }

    pub fn delete_session(&self, session_id: &str) -> crate::Result<()> {
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        guard.events.remove(session_id);
        guard.sessions.remove(session_id);
        drop(guard);
        let _ = Self::save_to_disk();
        Ok(())
    }

    pub fn delete_branch(&self, branch_id: &str) -> crate::Result<()> {
        let mut guard = GLOBAL_STORAGE.write().map_err(|e| crate::error::TimeLoopError::Storage(e.to_string()))?;
        guard.events.remove(branch_id);
        guard.branches.remove(branch_id);
        drop(guard);
        let _ = Self::save_to_disk();
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

    #[test]
    fn test_in_memory_storage() {
        let mut storage = Storage::new().unwrap();
        
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
}