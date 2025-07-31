use std::collections::HashMap;
use chrono::{DateTime, Utc};
use crate::Event;
use crate::session::Session;
use crate::branch::TimelineBranch;

pub struct Storage {
    events: HashMap<String, Vec<Event>>,
    sessions: HashMap<String, Session>,
    branches: HashMap<String, TimelineBranch>,
}

impl Storage {
    pub fn new() -> crate::Result<Self> {
        Ok(Self {
            events: HashMap::new(),
            sessions: HashMap::new(),
            branches: HashMap::new(),
        })
    }

    pub fn with_path(_path: &str) -> crate::Result<Self> {
        // In-memory storage ignores path
        Self::new()
    }

    pub fn get_db_path() -> crate::Result<std::path::PathBuf> {
        // Return a dummy path for compatibility
        Ok(std::path::PathBuf::from("/tmp/timeloop-memory"))
    }

    pub fn store_event(&mut self, event: &Event) -> crate::Result<()> {
        let session_events = self.events.entry(event.session_id.clone()).or_insert_with(Vec::new);
        session_events.push(event.clone());
        Ok(())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> crate::Result<Vec<Event>> {
        let events = self.events.get(session_id).cloned().unwrap_or_default();
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

    pub fn clear_session_events(&mut self, session_id: &str) -> crate::Result<()> {
        self.events.remove(session_id);
        Ok(())
    }

    // Session management
    pub fn store_session(&mut self, session: &Session) -> crate::Result<()> {
        self.sessions.insert(session.id.clone(), session.clone());
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> crate::Result<Option<Session>> {
        Ok(self.sessions.get(session_id).cloned())
    }

    pub fn list_sessions(&self) -> crate::Result<Vec<Session>> {
        let mut sessions: Vec<Session> = self.sessions.values().cloned().collect();
        sessions.sort_by_key(|s| s.created_at);
        Ok(sessions)
    }

    // Branch management
    pub fn store_branch(&mut self, branch: &TimelineBranch) -> crate::Result<()> {
        self.branches.insert(branch.id.clone(), branch.clone());
        Ok(())
    }

    pub fn get_branch(&self, branch_id: &str) -> crate::Result<Option<TimelineBranch>> {
        Ok(self.branches.get(branch_id).cloned())
    }

    pub fn list_branches(&self) -> crate::Result<Vec<TimelineBranch>> {
        let mut branches: Vec<TimelineBranch> = self.branches.values().cloned().collect();
        branches.sort_by_key(|b| b.created_at);
        Ok(branches)
    }

    pub fn delete_session(&mut self, session_id: &str) -> crate::Result<()> {
        // Delete all events for this session
        self.events.remove(session_id);
        
        // Delete the session itself
        self.sessions.remove(session_id);
        
        Ok(())
    }

    pub fn delete_branch(&mut self, branch_id: &str) -> crate::Result<()> {
        // Delete all events for this branch
        self.events.remove(branch_id);
        
        // Delete the branch metadata
        self.branches.remove(branch_id);
        
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