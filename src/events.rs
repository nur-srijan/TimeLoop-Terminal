use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use crate::storage::Storage;
use regex::Regex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FileChangeType {
    Created,
    Modified,
    Deleted,
    Renamed { old_path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// If true, command outputs will be redacted using the compiled patterns
    redact_output: bool,
    redact_patterns: Vec<Regex>,
}

impl EventRecorder {
    pub fn new(session_id: &str) -> crate::Result<Self> {
        let storage = Storage::new()?;
        // Initialize sequence counter from last stored event to avoid duplication when multiple recorders exist
        let last_seq = storage
            .get_last_event(session_id)?
            .map(|e| e.sequence_number)
            .unwrap_or(0);
        // Enable redaction by default with sensible patterns
        let default_patterns = vec![
            r"(?i)(password|pwd|secret|token|api_key)\s*[:=]\s*[^\s\n]+".to_string(),
            r"(?i)bearer\s+[A-Za-z0-9\-\._]+".to_string(),
        ];
        let compiled = default_patterns.into_iter().filter_map(|p| Regex::new(&p).ok()).collect();

        Ok(Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: last_seq,
            current_command: None,
            redact_output: true,
            redact_patterns: compiled,
        })
    }

    /// Disable redaction for this recorder. Useful for tests or when raw outputs are required.
    pub fn disable_redaction(&mut self) {
        self.redact_output = false;
        self.redact_patterns.clear();
    }

    /// Create an EventRecorder with redaction enabled. Patterns are optional; if
    /// none are provided a sensible default set is used.
    pub fn with_storage_and_redaction(session_id: &str, storage: Storage, redact: bool, patterns: Option<Vec<String>>) -> Self {
        let last_seq = storage
            .get_last_event(session_id)
            .ok()
            .flatten()
            .map(|e| e.sequence_number)
            .unwrap_or(0);

        let compiled = if redact {
            let pats = patterns.unwrap_or_else(|| vec![
                r"(?i)(password|pwd|secret|token|api_key)\s*[:=]\s*[^\s\n]+".to_string(),
                r"(?i)bearer\s+[A-Za-z0-9\-\._]+".to_string(),
            ]);
            pats.into_iter().filter_map(|p| Regex::new(&p).ok()).collect()
        } else { Vec::new() };

        Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: last_seq,
            current_command: None,
            redact_output: redact,
            redact_patterns: compiled,
        }
    }

    // Remove new_with_unique_db since we're using in-memory storage
    pub fn new_with_unique_db(session_id: &str) -> crate::Result<Self> {
        // In-memory storage doesn't need unique paths
        let mut s = Self::new(session_id)?;
        s.redact_output = false;
        s.redact_patterns = Vec::new();
        Ok(s)
    }

    pub fn with_storage(session_id: &str, storage: Storage) -> Self {
        let last_seq = storage
            .get_last_event(session_id)
            .ok()
            .flatten()
            .map(|e| e.sequence_number)
            .unwrap_or(0);
        Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: last_seq,
            current_command: None,
            redact_output: false,
            redact_patterns: Vec::new(),

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
        let stored_output = if self.redact_output {
            self.apply_redaction(output)
        } else {
            output.to_string()
        };
        
        let event = Event::new(
            &self.session_id,
            EventType::Command {
                command: command.to_string(),
                output: stored_output,
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

    fn apply_redaction(&self, text: &str) -> String {
        let mut s = text.to_string();
        for re in &self.redact_patterns {
            s = re.replace_all(&s, "[REDACTED]").to_string();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redaction() {
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("events_redaction.db");
        let storage = crate::storage::Storage::with_path(db_path.to_str().unwrap()).unwrap();
        let mut recorder = EventRecorder::with_storage_and_redaction("redact-session", storage, true, None);

        recorder.record_command("echo secret", "password=supersecret token=abc123", 0, "/tmp").unwrap();
        let events = recorder.get_events_for_session("redact-session").unwrap();
        assert_eq!(events.len(), 1);
        if let EventType::Command { output, .. } = &events[0].event_type {
            assert!(output.contains("[REDACTED]"));
            assert!(!output.contains("supersecret"));
            assert!(!output.contains("abc123"));
        } else { panic!("expected command event"); }
    }
}