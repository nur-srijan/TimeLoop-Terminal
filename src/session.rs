use crate::{EventType, Storage, TimeLoopError};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Debug, Clone, Serialize, Deserialize, Zeroize)]
pub struct Session {
    pub id: String,
    pub name: String,
    #[zeroize(skip)]
    pub created_at: DateTime<Utc>,
    #[zeroize(skip)]
    pub ended_at: Option<DateTime<Utc>>,
    pub parent_session_id: Option<String>,
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub name: String,
    pub duration: Duration,
    pub commands_executed: usize,
    pub files_modified: usize,
    pub last_command: String,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

pub struct SessionManager {
    storage: Storage,
}

impl SessionManager {
    pub fn new() -> crate::Result<Self> {
        let storage = Storage::new()?;
        Ok(Self { storage })
    }

    pub fn with_storage(storage: Storage) -> Self {
        Self { storage }
    }

    pub fn create_session(&mut self, name: &str) -> crate::Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let session = Session {
            id: session_id.clone(),
            name: name.to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };

        self.storage.store_session(&session)?;
        Ok(session_id)
    }

    pub fn create_branch(
        &mut self,
        parent_session_id: &str,
        branch_name: &str,
    ) -> crate::Result<String> {
        // Verify parent session exists
        let parent_session = self
            .get_session(parent_session_id)?
            .ok_or_else(|| TimeLoopError::SessionNotFound(parent_session_id.to_string()))?;

        let branch_id = Uuid::new_v4().to_string();
        let branch_session = Session {
            id: branch_id.clone(),
            name: format!("{} (branch: {})", parent_session.name, branch_name),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: Some(parent_session_id.to_string()),
            branch_name: Some(branch_name.to_string()),
        };

        self.storage.store_session(&branch_session)?;
        Ok(branch_id)
    }

    pub fn end_session(&mut self, session_id: &str) -> crate::Result<()> {
        if let Some(mut session) = self.get_session(session_id)? {
            session.ended_at = Some(Utc::now());
            self.storage.store_session(&session)?;
        }
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> crate::Result<Option<Session>> {
        self.storage.get_session(session_id)
    }

    pub fn list_sessions(&self) -> crate::Result<Vec<Session>> {
        self.storage.list_sessions()
    }

    pub fn get_session_summary(&self, session_id: &str) -> crate::Result<SessionSummary> {
        let session = self
            .get_session(session_id)?
            .ok_or_else(|| TimeLoopError::SessionNotFound(session_id.to_string()))?;

        let events = self.storage.get_events_for_session(session_id)?;

        let mut commands_executed = 0;
        let mut files_modified = 0;
        let mut last_command = String::new();

        for event in &events {
            match &event.event_type {
                EventType::Command { command, .. } => {
                    commands_executed += 1;
                    last_command = command.clone();
                }
                EventType::FileChange { .. } => {
                    files_modified += 1;
                }
                _ => {}
            }
        }

        let duration = if let Some(ended_at) = session.ended_at {
            ended_at - session.created_at
        } else {
            Utc::now() - session.created_at
        };

        Ok(SessionSummary {
            session_id: session.id,
            name: session.name,
            duration,
            commands_executed,
            files_modified,
            last_command,
            created_at: session.created_at,
            ended_at: session.ended_at,
        })
    }

    pub fn delete_session(&mut self, session_id: &str) -> crate::Result<()> {
        self.storage.delete_session(session_id)
    }

    pub fn get_session_tree(&self) -> crate::Result<Vec<SessionNode>> {
        let sessions = self.list_sessions()?;
        let mut tree = Vec::new();
        let mut session_map = std::collections::HashMap::new();

        // Create a map of sessions by ID
        for session in &sessions {
            session_map.insert(&session.id, session);
        }

        // Build the tree
        for session in &sessions {
            if session.parent_session_id.is_none() {
                tree.push(self.build_session_node(session, &session_map));
            }
        }

        Ok(tree)
    }

    fn build_session_node(
        &self,
        session: &Session,
        session_map: &std::collections::HashMap<&String, &Session>,
    ) -> SessionNode {
        let mut children = Vec::new();

        for other_session in session_map.values() {
            if other_session.parent_session_id.as_ref() == Some(&session.id) {
                children.push(self.build_session_node(other_session, session_map));
            }
        }

        SessionNode {
            session: session.clone(),
            children,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionNode {
    pub session: Session,
    pub children: Vec<SessionNode>,
}
