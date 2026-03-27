use timeloop_terminal::{Storage, Event, EventType};
use tempfile::TempDir;
use chrono::Utc;
use uuid::Uuid;
use std::time::Instant;

fn main() {
    let tmp_dir = TempDir::new().unwrap();
    let state_file = tmp_dir.path().join("state.json");
    let mut storage = Storage::with_path_and_format(state_file.to_str().unwrap(), timeloop_terminal::storage::PersistenceFormat::Json).unwrap();
    storage.enable_append_only();

    let session_id = "bench-session";
    let iterations = 1000;

    let start = Instant::now();
    for i in 0..iterations {
        let event = Event {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            event_type: EventType::KeyPress {
                key: "a".to_string(),
                timestamp: Utc::now(),
            },
            sequence_number: i as u64,
            timestamp: Utc::now(),
        };
        storage.store_event(&event).unwrap();
    }
    let duration = start.elapsed();
    println!("Time to store {} events: {:?}", iterations, duration);
    println!("Average time per event: {:?}", duration / iterations as u32);
}
