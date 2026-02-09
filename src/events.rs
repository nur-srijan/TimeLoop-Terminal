use crate::storage::Storage;
use chrono::{DateTime, Utc};
use regex::Regex;
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::Path;
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Zeroize)]
pub enum EventType {
    /// A keystroke event
    KeyPress {
        key: String,
        #[zeroize(skip)]
        timestamp: DateTime<Utc>,
    },
    /// A command execution
    Command {
        command: String,
        output: String,
        exit_code: i32,
        working_directory: String,
        #[zeroize(skip)]
        timestamp: DateTime<Utc>,
    },
    /// File system change
    FileChange {
        path: String,
        change_type: FileChangeType,
        content_hash: Option<String>,
        #[zeroize(skip)]
        timestamp: DateTime<Utc>,
    },
    /// Terminal state change
    TerminalState {
        cursor_position: (u16, u16),
        screen_size: (u16, u16),
        #[zeroize(skip)]
        timestamp: DateTime<Utc>,
    },
    /// Session metadata
    SessionMetadata {
        name: String,
        #[zeroize(skip)]
        created_at: DateTime<Utc>,
        #[zeroize(skip)]
        timestamp: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Zeroize)]
pub enum FileChangeType {
    Created,
    Modified,
    Deleted,
    Renamed { old_path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Zeroize)]
pub struct Event {
    pub id: String,
    pub session_id: String,
    pub event_type: EventType,
    pub sequence_number: u64,
    #[zeroize(skip)]
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
    redact_literals: Vec<String>,
    is_paused: bool,
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
            // Generic assignment (e.g. password=...)
            r"(?i)(password|pwd|secret|token|api_key|access_key|secret_key)\s*[:=]\s*[^\s\n]+".to_string(),
            // Bearer tokens
            r"(?i)bearer\s+[A-Za-z0-9\-\._]+".to_string(),
            // AWS Access Key ID
            r"(?i)AKIA[0-9A-Z]{16}".to_string(),
            // AWS Secret Access Key
            r"(?i)[0-9a-zA-Z/+]{40}".to_string(),
            // GitHub Token
            r"(?i)gh[pousr]_[A-Za-z0-9_]{36,255}".to_string(),
            // Slack Token
            r"(?i)xox[baprs]-([0-9a-zA-Z]{10,48})?".to_string(),
            // Private Key Header
            r"(?i)-----BEGIN[ A-Z0-9]+PRIVATE KEY-----".to_string(),
            // Generic URI with credentials
            r"(?i)[a-z]+://[^/\s]*:[^/\s]*@[^/\s]+".to_string(),
        ];
        let compiled = default_patterns
            .into_iter()
            .filter_map(|p| Regex::new(&p).ok())
            .collect();

        let mut recorder = Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: last_seq,
            current_command: None,
            redact_output: true,
            redact_patterns: compiled,
            redact_literals: Vec::new(),
            is_paused: false,
        };
        recorder.load_env_secrets();
        Ok(recorder)
    }

    /// Disable redaction for this recorder. Useful for tests or when raw outputs are required.
    pub fn disable_redaction(&mut self) {
        self.redact_output = false;
        self.redact_patterns.clear();
    }

    /// Create an EventRecorder with redaction enabled. Patterns are optional; if
    /// none are provided a sensible default set is used.
    pub fn with_storage_and_redaction(
        session_id: &str,
        storage: Storage,
        redact: bool,
        patterns: Option<Vec<String>>,
    ) -> Self {
        let last_seq = storage
            .get_last_event(session_id)
            .ok()
            .flatten()
            .map(|e| e.sequence_number)
            .unwrap_or(0);

        let compiled = if redact {
            let pats = patterns.unwrap_or_else(|| {
                vec![
                    r"(?i)(password|pwd|secret|token|api_key)\s*[:=]\s*[^\s\n]+".to_string(),
                    r"(?i)bearer\s+[A-Za-z0-9\-\._]+".to_string(),
                ]
            });
            pats.into_iter()
                .filter_map(|p| Regex::new(&p).ok())
                .collect()
        } else {
            Vec::new()
        };

        let mut recorder = Self {
            session_id: session_id.to_string(),
            storage,
            sequence_counter: last_seq,
            current_command: None,
            redact_output: redact,
            redact_patterns: compiled,
            redact_literals: Vec::new(),
            is_paused: false,
        };
        if redact {
            recorder.load_env_secrets();
        }
        recorder
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
            redact_literals: Vec::new(),
            is_paused: false,
        }
    }

    /// Pause recording (Incognito Mode)
    pub fn pause_recording(&mut self) {
        self.is_paused = true;
    }

    /// Resume recording
    pub fn resume_recording(&mut self) {
        self.is_paused = false;
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused
    }

    pub fn record_key_press(&mut self, key: &str) -> crate::Result<()> {
        if self.is_paused {
            return Ok(());
        }
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

    pub fn record_command(
        &mut self,
        command: &str,
        output: &str,
        exit_code: i32,
        working_dir: &str,
    ) -> crate::Result<()> {
        if self.is_paused {
            return Ok(());
        }
        self.sequence_counter += 1;
        let (stored_command, stored_output) = if self.redact_output {
            (self.apply_redaction(command), self.apply_redaction(output))
        } else {
            (command.to_string(), output.to_string())
        };

        let event = Event::new(
            &self.session_id,
            EventType::Command {
                command: stored_command,
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

    pub fn record_file_change(
        &mut self,
        path: &str,
        change_type: FileChangeType,
    ) -> crate::Result<()> {
        if self.is_paused {
            return Ok(());
        }
        self.sequence_counter += 1;

        // Compute content hash if the file exists and isn't deleted
        let content_hash = if change_type != FileChangeType::Deleted {
            self.compute_file_hash(path)
        } else {
            None
        };

        let event = Event::new(
            &self.session_id,
            EventType::FileChange {
                path: path.to_string(),
                change_type,
                content_hash,
                timestamp: Utc::now(),
            },
            self.sequence_counter,
        );

        self.storage.store_event(&event)?;
        Ok(())
    }

    pub fn record_terminal_state(
        &mut self,
        cursor_pos: (u16, u16),
        screen_size: (u16, u16),
    ) -> crate::Result<()> {
        if self.is_paused {
            return Ok(());
        }
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

    pub fn get_events_in_range(
        &self,
        session_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> crate::Result<Vec<Event>> {
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

    /// Load secrets from environment variables to be redacted as literal strings
    pub fn load_env_secrets(&mut self) {
        for (key, value) in std::env::vars() {
            // Heuristic to identify secret keys
            let key_upper = key.to_uppercase();
            if (key_upper.contains("KEY") ||
                key_upper.contains("TOKEN") ||
                key_upper.contains("SECRET") ||
                key_upper.contains("PASSWORD")) &&
                !value.is_empty() && value.len() > 3 { // Avoid redacting short common strings
                self.redact_literals.push(value);
            }
        }
        // Sort by length descending to replace longest matches first
        self.redact_literals.sort_by_key(|b| std::cmp::Reverse(b.len()));
    }

    fn apply_redaction(&self, text: &str) -> String {
        let mut s = text.to_string();

        // First pass: Env var literals
        for secret in &self.redact_literals {
            s = s.replace(secret, "[REDACTED_ENV]");
        }

        // Second pass: Regex patterns
        for re in &self.redact_patterns {
            s = re.replace_all(&s, "[REDACTED]").to_string();
        }
        s
    }

    fn compute_file_hash(&self, path: &str) -> Option<String> {
        let path = Path::new(path);
        if !path.exists() {
            return None;
        }

        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return None,
        };
        let mut reader = std::io::BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192]; // 8KB buffer

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buffer[..n]),
                Err(_) => return None,
            }
        }

        let result = hasher.finalize();
        Some(format!("{:x}", result))
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
        let mut recorder =
            EventRecorder::with_storage_and_redaction("redact-session", storage, true, None);

        recorder
            .record_command(
                "echo secret",
                "password=supersecret token=abc123",
                0,
                "/tmp",
            )
            .unwrap();
        let events = recorder.get_events_for_session("redact-session").unwrap();
        assert_eq!(events.len(), 1);
        if let EventType::Command { output, .. } = &events[0].event_type {
            assert!(output.contains("[REDACTED]"));
            assert!(!output.contains("supersecret"));
            assert!(!output.contains("abc123"));
        } else {
            panic!("expected command event");
        }
    }

    #[test]
    fn test_file_hashing() {
        use std::io::Write;

        let tmp_dir = tempfile::TempDir::new().unwrap();
        let file_path = tmp_dir.path().join("test_file.txt");
        let db_path = tmp_dir.path().join("events_hashing.db");

        // Create a test file
        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(b"Hello world").unwrap();

        let storage = crate::storage::Storage::with_path(db_path.to_str().unwrap()).unwrap();
        let mut recorder = EventRecorder::with_storage("hash-session", storage);

        recorder.record_file_change(
            file_path.to_str().unwrap(),
            FileChangeType::Modified
        ).unwrap();

        let events = recorder.get_events_for_session("hash-session").unwrap();
        assert_eq!(events.len(), 1);

        if let EventType::FileChange { content_hash, .. } = &events[0].event_type {
            assert!(content_hash.is_some());
            // SHA256 of "Hello world"
            assert_eq!(content_hash.as_ref().unwrap(), "64ec88ca00b268e5ba1a35678a1b5316d212f4f366b2477232534a8aeca37f3c");
        } else {
            panic!("expected file change event");
        }
    }
}
