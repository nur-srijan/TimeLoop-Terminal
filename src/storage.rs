use sled;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use crate::{Event, TimeLoopError};

pub struct Storage {
    pub(crate) db: sled::Db,
}

impl Storage {
    pub fn new() -> crate::Result<Self> {
        let db_path = Self::get_db_path()?;
        let db = sled::open(db_path)?;
        Ok(Self { db })
    }

    pub fn with_path(path: &str) -> crate::Result<Self> {
        let db = sled::open(path)?;
        Ok(Self { db })
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

// Re-export types that are used in this module
use crate::session::Session;
use crate::branch::TimelineBranch; 