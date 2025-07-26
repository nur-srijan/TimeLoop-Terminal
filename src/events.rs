use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use std::collections::HashMap;
use crate::storage::Storage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    /// A keystroke event
    KeyPress {
        key: String,
        timestamp: DateTime<Utc>,
    },
    /// A command execution
    Command {
        command: String,
        output: String,
        exit_code: i32,
        working_directory: String,
        timestamp: DateTime<Utc>,
    },
    /// File system change
    FileChange {
        path: String,
        change_type: FileChangeType,
        content_hash: Option<String>,
        timestamp: DateTime<Utc>,
    },
    /// Terminal state change
    TerminalState {
        cursor_position: (u16, u16),
        screen_size: (u16, u16),
        timestamp: DateTime<Utc>,
    },
    /// Session metadata
    SessionMetadata {
        name: String,
        created_at: DateTime<Utc>,
        timestamp: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileChangeType {
    Created,
    Modified,
    Deleted,
    Renamed { old_path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub session_id: String,
    pub event_type: EventType,
    pub sequence_number: u64,
    pub timestamp: DateTime<Utc>,
}

impl Event {
    pub fn new(session_id: &str, event_type: EventType, sequence_number: u64) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            event_type,
            sequence_number,
            timestamp: Utc::now(),
        }
    }
}

pub struct EventRecorder {
    session_id: String,
    storage: Storage,
    sequence_counter: u64,
    current_command: Option<String>,
    command_buffer: Vec<String>,
}

impl EventRecorder {
    pub fn new(session_id: &str) -> crate::Result<Self> {
        let storage = Storage::new()?;
        Ok(Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: 0,
            current_command: None,
            command_buffer: Vec::new(),
        })
    }

    /// Create a new event recorder with a unique database path to avoid conflicts
    pub fn new_with_unique_db(session_id: &str) -> crate::Result<Self> {
        let mut db_path = Storage::get_db_path()?;
        // Modify the path to be unique for this instance
        let filename = db_path.file_name().unwrap().to_string_lossy();
        let new_filename = format!("{}_{}", session_id, filename);
        db_path.set_file_name(new_filename);
        
        let storage = Storage::with_path(db_path.to_str().unwrap())?;
        Ok(Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: 0,
            current_command: None,
            command_buffer: Vec::new(),
        })
    }

    pub fn with_storage(session_id: &str, storage: Storage) -> Self {
        Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: 0,
            current_command: None,
            command_buffer: Vec::new(),
        }
    }

    pub fn record_key_press(&mut self, key: &str) -> crate::Result<()> {
        self.sequence_counter += 1;
        let event = Event::new(
            &self.session_id,
            EventType::KeyPress {
                key: key.to_string(),
                timestamp: Utc::now(),
            },
            self.sequence_counter,
        );
        
        self.storage.store_event(&event)?;
        Ok(())
    }

    pub fn record_command(&mut self, command: &str, output: &str, exit_code: i32, working_dir: &str) -> crate::Result<()> {
        self.sequence_counter += 1;
        let event = Event::new(
            &self.session_id,
            EventType::Command {
                command: command.to_string(),
                output: output.to_string(),
                exit_code,
                working_directory: working_dir.to_string(),
                timestamp: Utc::now(),
            },
            self.sequence_counter,
        );
        
        self.storage.store_event(&event)?;
        self.current_command = None;
        Ok(())
    }

    pub fn record_file_change(&mut self, path: &str, change_type: FileChangeType) -> crate::Result<()> {
        self.sequence_counter += 1;
        let event = Event::new(
            &self.session_id,
            EventType::FileChange {
                path: path.to_string(),
                change_type,
                content_hash: None, // TODO: Implement content hashing
                timestamp: Utc::now(),
            },
            self.sequence_counter,
        );
        
        self.storage.store_event(&event)?;
        Ok(())
    }

    pub fn record_terminal_state(&mut self, cursor_pos: (u16, u16), screen_size: (u16, u16)) -> crate::Result<()> {
        self.sequence_counter += 1;
        let event = Event::new(
            &self.session_id,
            EventType::TerminalState {
                cursor_position: cursor_pos,
                screen_size,
                timestamp: Utc::now(),
            },
            self.sequence_counter,
        );
        
        self.storage.store_event(&event)?;
        Ok(())
    }

    pub fn get_events_for_session(&self, session_id: &str) -> crate::Result<Vec<Event>> {
        self.storage.get_events_for_session(session_id)
    }

    pub fn get_events_in_range(&self, session_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> crate::Result<Vec<Event>> {
        self.storage.get_events_in_range(session_id, start, end)
    }

    pub fn get_last_event(&self, session_id: &str) -> crate::Result<Option<Event>> {
        self.storage.get_last_event(session_id)
    }

    pub fn clear_session_events(&mut self, session_id: &str) -> crate::Result<()> {
        self.storage.clear_session_events(session_id)
    }

    /// Get the session ID for this event recorder
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get a reference to the storage
    pub fn storage(&self) -> &Storage {
        &self.storage
    }
} 