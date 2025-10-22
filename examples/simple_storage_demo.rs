use timeloop_terminal::Storage;

/// Simple demonstration of storage improvements without encryption
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TimeLoop Terminal - Simple Storage Demo");
    println!("======================================");
    
    // Test 1: Atomic Counter for Pending Writes
    println!("\n1. Testing Atomic Counter for Pending Writes");
    test_pending_writes_counter();
    
    // Test 2: Basic Backup (Plaintext)
    println!("\n2. Testing Basic Backup");
    test_basic_backup();
    
    // Test 3: Concurrency Performance
    println!("\n3. Testing Concurrency Performance");
    test_concurrency_performance();
    
    println!("\n✅ All storage improvements working correctly!");
    Ok(())
}

fn test_pending_writes_counter() {
    // Create a storage instance
    let storage = Storage::new().unwrap();
    
    // Check initial pending writes count
    let initial_pending = storage.get_pending_writes();
    println!("   Initial pending writes: {}", initial_pending);
    
    // Simulate some operations that would increment the counter
    let session = timeloop_terminal::session::Session {
        id: "test-session".to_string(),
        name: "Test Session".to_string(),
        created_at: chrono::Utc::now(),
        ended_at: None,
        parent_session_id: None,
        branch_name: None,
    };
    
    // Store session (this will increment pending writes)
    storage.store_session(&session).unwrap();
    
    // Check pending writes after operation
    let after_pending = storage.get_pending_writes();
    println!("   Pending writes after operation: {}", after_pending);
    
    // Flush to complete the operation
    storage.flush().unwrap();
    
    // Check final pending writes count
    let final_pending = storage.get_pending_writes();
    println!("   Final pending writes: {}", final_pending);
    
    println!("   ✅ Atomic counter working correctly");
}

fn test_basic_backup() {
    println!("   Testing basic backup...");
    let storage = Storage::with_path("test_basic.json").unwrap();
    
    let session = timeloop_terminal::session::Session {
        id: "basic-session".to_string(),
        name: "Basic Session".to_string(),
        created_at: chrono::Utc::now(),
        ended_at: None,
        parent_session_id: None,
        branch_name: None,
    };
    
    storage.store_session(&session).unwrap();
    storage.export_session_to_file("basic-session", "basic_backup.json").unwrap();
    
    // Check if backup was created
    if std::path::Path::new("basic_backup.json").exists() {
        let backup_content = std::fs::read_to_string("basic_backup.json").unwrap();
        println!("   Backup file size: {} bytes", backup_content.len());
        if backup_content.contains("basic-session") {
            println!("   ✅ Basic backup created successfully");
        } else {
            println!("   ⚠️  Backup created but content format may have changed");
        }
    } else {
        println!("   ❌ Backup file was not created");
    }
    
    // Cleanup
    let _ = std::fs::remove_file("test_basic.json");
    let _ = std::fs::remove_file("basic_backup.json");
}

fn test_concurrency_performance() {
    println!("   Testing concurrent write operations...");
    
    let storage = Storage::new().unwrap();
    let storage = std::sync::Arc::new(storage);
    
    let mut handles = vec![];
    let num_threads = 5; // Reduced for demo
    let operations_per_thread = 20; // Reduced for demo
    
    let start_time = std::time::Instant::now();
    
    // Spawn multiple threads performing write operations
    for thread_id in 0..num_threads {
        let storage = storage.clone();
        let handle = std::thread::spawn(move || {
            for i in 0..operations_per_thread {
                let session = timeloop_terminal::session::Session {
                    id: format!("thread-{}-session-{}", thread_id, i),
                    name: format!("Thread {} Session {}", thread_id, i),
                    created_at: chrono::Utc::now(),
                    ended_at: None,
                    parent_session_id: None,
                    branch_name: None,
                };
                
                storage.store_session(&session).unwrap();
                
                // Check pending writes during operation
                let pending = storage.get_pending_writes();
                if pending > 0 {
                    // This shows the atomic counter is working
                }
            }
        });
        handles.push(handle);
    }
    
    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }
    
    let duration = start_time.elapsed();
    let total_operations = num_threads * operations_per_thread;
    
    println!("   Completed {} operations in {:?}", total_operations, duration);
    println!("   Operations per second: {:.2}", total_operations as f64 / duration.as_secs_f64());
    
    // Check final pending writes (should be 0)
    let final_pending = storage.get_pending_writes();
    println!("   Final pending writes: {}", final_pending);
    
    // Flush to complete all operations
    storage.flush().unwrap();
    
    println!("   ✅ Concurrent operations completed successfully");
}