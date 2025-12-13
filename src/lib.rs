#[cfg(feature = "ai")]
pub mod ai;
pub mod branch;
pub mod error;
pub mod events;
pub mod file_watcher;
pub mod replay;
pub mod session;
pub mod storage;
pub mod terminal;

pub use branch::{BranchManager, TimelineBranch};
pub use error::TimeLoopError;
pub use events::{Event, EventRecorder, EventType, FileChangeType};
pub use replay::ReplayEngine;
pub use session::{Session, SessionManager, SessionSummary};
pub use storage::Storage;
pub use terminal::TerminalEmulator;

/// Re-export commonly used types
pub type Result<T> = std::result::Result<T, TimeLoopError>;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    #[test]
    fn test_session_creation() {
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("events.db");
        let storage = Storage::with_path(db_path.to_str().unwrap()).unwrap();
        let mut session_manager = SessionManager::with_storage(storage);
        let session_id = session_manager.create_session("test-session").unwrap();
        assert!(!session_id.is_empty());

        let session = session_manager.get_session(&session_id).unwrap().unwrap();
        assert_eq!(session.name, "test-session");
        assert_eq!(session.id, session_id);
    }

    #[test]
    fn test_event_recording() {
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("events.db");
        let storage = Storage::with_path(db_path.to_str().unwrap()).unwrap();
        let event_recorder = EventRecorder::with_storage("test-session", storage);
        let events = event_recorder
            .get_events_for_session("test-session")
            .unwrap();
        assert_eq!(events.len(), 0); // Should be empty initially
    }

    #[test]
    fn test_storage_operations() {
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("events.db");
        let storage = Storage::with_path(db_path.to_str().unwrap()).unwrap();
        let session = Session {
            id: "test-id".to_string(),
            name: "test-session".to_string(),
            created_at: Utc::now(),
            ended_at: None,
            parent_session_id: None,
            branch_name: None,
        };

        storage.store_session(&session).unwrap();
        let retrieved = storage.get_session("test-id").unwrap().unwrap();
        assert_eq!(retrieved.name, session.name);
    }
}
