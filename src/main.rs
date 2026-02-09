use chrono::Utc;
use clap::{Parser, Subcommand};
use timeloop_terminal::{
    error::TimeLoopError, events::EventRecorder, replay::ReplayEngine, session::SessionManager,
    storage::Storage, terminal::TerminalEmulator,
};
use tracing::info;

#[derive(Parser)]
#[command(name = "timeloop")]
#[command(about = "Your terminal is not just a tool. It's a time machine.")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Start a new session with optional name
    #[arg(short, long)]
    session: Option<String>,

    /// Enable replay mode
    #[arg(short, long)]
    replay: bool,

    /// Persistence format (json|cbor)
    #[arg(long)]
    persistence_format: Option<String>,

    /// Enable append-only event logging (deprecated, enabled by default)
    #[arg(long)]
    append_events: bool,

    /// Disable append-only event logging (force full rewrite persistence)
    #[arg(long)]
    disable_append_events: bool,

    /// Argon2 memory in KiB (default: 65536)
    #[arg(long)]
    argon2_memory_kib: Option<u32>,

    /// Argon2 iterations / time cost (default: 3)
    #[arg(long)]
    argon2_iterations: Option<u32>,

    /// Argon2 parallelism (default: 1)
    #[arg(long)]
    argon2_parallelism: Option<u32>,

    /// Max append log size in MB before rotation
    #[arg(long)]
    max_log_size_mb: Option<u64>,

    /// Max events before rotation
    #[arg(long)]
    max_events: Option<usize>,

    /// Retention count for rotated logs
    #[arg(long)]
    retention_count: Option<usize>,

    /// Background compaction interval in seconds
    #[arg(long)]
    compaction_interval_secs: Option<u64>,

    /// Branch from a specific session ID
    #[arg(short, long)]
    branch: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a new terminal session
    Start {
        /// Session name
        #[arg(short, long)]
        name: Option<String>,
    },
    /// List all sessions
    List,
    /// Replay a session
    Replay {
        /// Session ID to replay
        session_id: String,
        /// Playback speed (1.0 = normal, 2.0 = 2x speed)
        #[arg(short, long, default_value = "1.0")]
        speed: f32,
    },
    /// Replay a session within a time range
    ReplayRange {
        /// Session ID to replay
        session_id: String,
        /// ISO8601 start timestamp
        #[arg(long)]
        start: String,
        /// ISO8601 end timestamp
        #[arg(long)]
        end: String,
        /// Playback speed
        #[arg(short, long, default_value = "1.0")]
        speed: f32,
    },
    /// Create a branch from a session
    Branch {
        /// Source session ID
        session_id: String,
        /// Branch name
        name: String,
        /// Create branch at specific event ID
        #[arg(long)]
        event_id: Option<String>,
        /// Create branch at timestamp (RFC3339)
        #[arg(long)]
        at: Option<String>,
    },
    /// List branches for a session
    Branches {
        /// Session ID
        session_id: String,
    },
    /// Merge a branch into a target session
    Merge {
        /// Branch ID
        branch_id: String,
        /// Target session ID
        target_session_id: String,
    },
    /// Delete a branch
    DeleteBranch {
        /// Branch ID to delete
        branch_id: String,
    },
    /// Show session tree (parent/child relationships)
    Tree,
    /// Show event timeline for a session
    Timeline {
        /// Session ID
        session_id: String,
    },
    /// Show session summary
    Summary {
        /// Session ID
        session_id: String,
    },
    /// Export a session to JSON file
    Export {
        /// Session ID
        session_id: String,
        /// Output file path
        output: String,
    },
    /// Import a session from JSON file
    Import {
        /// Input file path
        input: String,
    },
    /// AI summarize a session (requires --features ai)
    Summarize {
        /// Session ID to summarize
        session_id: String,
        /// Model name (default: openrouter/auto)
        #[arg(long)]
        model: Option<String>,
    },
    /// Compact storage
    Compact {
        /// Optional path to a storage file to compact (defaults to global storage)
        #[arg(long)]
        file: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), TimeLoopError> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("🚀 Starting TimeLoop Terminal...");

    let cli = Cli::parse();

    // Apply persistence-related global flags early so Storage::new() uses them
    if let Some(fmt) = &cli.persistence_format {
        match fmt.as_str() {
            "json" => timeloop_terminal::storage::Storage::set_global_persistence_format(
                timeloop_terminal::storage::PersistenceFormat::Json,
            ),
            "cbor" => timeloop_terminal::storage::Storage::set_global_persistence_format(
                timeloop_terminal::storage::PersistenceFormat::Cbor,
            ),
            other => {
                return Err(TimeLoopError::Configuration(format!(
                    "Unknown persistence format: {}",
                    other
                )))
            }
        }
    }
    // Append-only is now enabled by default. The --append-events flag is kept for backward
    // compatibility (it's a no-op as the feature is on). To disable, use --disable-append-events.
    let append_only = !cli.disable_append_events;
    timeloop_terminal::storage::Storage::set_global_append_only(append_only);

    // Wire Argon2 CLI params into global config if provided
    if cli.argon2_memory_kib.is_some()
        || cli.argon2_iterations.is_some()
        || cli.argon2_parallelism.is_some()
    {
        let mut cfg = timeloop_terminal::storage::Argon2Config::default();
        if let Some(m) = cli.argon2_memory_kib {
            cfg.memory_kib = m;
        }
        if let Some(i) = cli.argon2_iterations {
            cfg.iterations = i;
        }
        if let Some(p) = cli.argon2_parallelism {
            cfg.parallelism = p;
        }
        timeloop_terminal::storage::Storage::set_global_argon2_config(cfg);
    }

    // Wire compaction/rotation CLI flags into global compaction policy
    if cli.max_log_size_mb.is_some()
        || cli.max_events.is_some()
        || cli.retention_count.is_some()
        || cli.compaction_interval_secs.is_some()
    {
        let mut pol = timeloop_terminal::storage::CompactionPolicy::default();
        if let Some(mb) = cli.max_log_size_mb {
            pol.max_log_size_bytes = Some(mb * 1024 * 1024);
        }
        if let Some(me) = cli.max_events {
            pol.max_events = Some(me);
        }
        if let Some(rc) = cli.retention_count {
            pol.retention_count = rc;
        }
        if let Some(ci) = cli.compaction_interval_secs {
            pol.compaction_interval_secs = Some(ci);
        }
        timeloop_terminal::storage::Storage::set_global_compaction_policy(pol);
    }

    match &cli.command {
        Some(Commands::Start { name }) => {
            let session_name = name.as_deref().unwrap_or("default");
            start_session(session_name).await?;
        }
        Some(Commands::List) => {
            list_sessions().await?;
        }
        Some(Commands::Replay { session_id, speed }) => {
            replay_session(session_id, *speed).await?;
        }
        Some(Commands::ReplayRange {
            session_id,
            start,
            end,
            speed,
        }) => {
            replay_session_range(session_id, start, end, *speed).await?;
        }
        Some(Commands::Branch {
            session_id,
            name,
            event_id,
            at,
        }) => {
            create_branch(session_id, name, event_id.as_deref(), at.as_deref()).await?;
        }
        Some(Commands::Branches { session_id }) => {
            list_branches(session_id).await?;
        }
        Some(Commands::Merge {
            branch_id,
            target_session_id,
        }) => {
            merge_branch(branch_id, target_session_id).await?;
        }
        Some(Commands::DeleteBranch { branch_id }) => {
            delete_branch(branch_id).await?;
        }
        Some(Commands::Summary { session_id }) => {
            show_summary(session_id).await?;
        }
        Some(Commands::Tree) => {
            show_session_tree().await?;
        }
        Some(Commands::Timeline { session_id }) => {
            show_event_timeline(session_id).await?;
        }
        Some(Commands::Export { session_id, output }) => {
            export_session(session_id, output).await?;
        }
        Some(Commands::Import { input }) => {
            import_session(input).await?;
        }
        Some(Commands::Summarize { session_id, model }) => {
            run_ai_summarize(session_id, model.as_deref()).await?;
        }
        Some(Commands::Compact { file }) => {
            // If a file was provided, compact that specific storage instance; otherwise compact global storage.
            if let Some(f) = file {
                let st = timeloop_terminal::storage::Storage::with_path(f.as_str())?;
                st.compact()?;
                println!("Compacted storage at {}", f);
            } else {
                let st = timeloop_terminal::storage::Storage::new()?;
                st.compact()?;
                println!("Compacted global storage");
            }
        }
        None => {
            // Default behavior: start a new session
            let session_name = cli.session.as_deref().unwrap_or("default");
            start_session(session_name).await?;
        }
    }

    Ok(())
}

async fn start_session(name: &str) -> Result<(), TimeLoopError> {
    info!("🎬 Starting new session: {}", name);

    let _storage = Storage::new()?;

    let mut session_manager = SessionManager::new()?;
    let session_id = session_manager.create_session(name)?;

    let event_recorder = EventRecorder::new(&session_id)?;
    let mut terminal = TerminalEmulator::new(event_recorder)?;

    info!("📝 Session {} started with ID: {}", name, session_id);

    // Start the terminal emulator
    terminal.run().await?;

    Ok(())
}

async fn list_sessions() -> Result<(), TimeLoopError> {
    info!("📋 Listing all sessions...");

    let session_manager = SessionManager::new()?;
    let sessions = session_manager.list_sessions()?;

    println!("🕰️  TimeLoop Sessions:");
    println!("{}", "─".repeat(50));

    for session in sessions {
        println!(
            "📁 {} - {} ({})",
            session.id,
            session.name,
            session.created_at.format("%Y-%m-%d %H:%M:%S")
        );
    }

    Ok(())
}

async fn replay_session(session_id: &str, speed: f32) -> Result<(), TimeLoopError> {
    info!("🎥 Replaying session: {} at {}x speed", session_id, speed);

    let replay_engine = ReplayEngine::new(session_id)?;
    replay_engine.replay(speed).await?;

    Ok(())
}

async fn replay_session_range(
    session_id: &str,
    start: &str,
    end: &str,
    speed: f32,
) -> Result<(), TimeLoopError> {
    let replay_engine = ReplayEngine::new(session_id)?;
    let start_ts = chrono::DateTime::parse_from_rfc3339(start)
        .map_err(|e| TimeLoopError::Replay(format!("Invalid start time: {}", e)))?
        .with_timezone(&Utc);
    let end_ts = chrono::DateTime::parse_from_rfc3339(end)
        .map_err(|e| TimeLoopError::Replay(format!("Invalid end time: {}", e)))?
        .with_timezone(&Utc);
    replay_engine.replay_range(start_ts, end_ts, speed).await?;
    Ok(())
}

async fn create_branch(
    session_id: &str,
    name: &str,
    event_id: Option<&str>,
    at: Option<&str>,
) -> Result<(), TimeLoopError> {
    info!("🧬 Creating branch '{}' from session: {}", name, session_id);

    // Create a branch session entry
    let mut session_manager = SessionManager::new()?;
    let new_session_id = session_manager.create_branch(session_id, name)?;

    // Also record a timeline branch at the last event as branch point
    let storage = Storage::new()?;
    let branch_point_id = if let Some(eid) = event_id {
        eid.to_string()
    } else if let Some(at_ts) = at {
        let ts = chrono::DateTime::parse_from_rfc3339(at_ts)
            .map_err(|e| TimeLoopError::Branch(format!("Invalid --at timestamp: {}", e)))?
            .with_timezone(&Utc);
        let mut events = storage.get_events_for_session(session_id)?;
        events.sort_by_key(|e| e.timestamp);
        events
            .iter()
            .rev()
            .find(|e| e.timestamp <= ts)
            .map(|e| e.id.clone())
            .unwrap_or_else(|| "0".to_string())
    } else {
        storage
            .get_last_event(session_id)?
            .map(|e| e.id)
            .unwrap_or_else(|| "0".to_string())
    };

    let mut branch_manager = timeloop_terminal::branch::BranchManager::new()?;
    let timeline_branch_id =
        branch_manager.create_branch(session_id, name, &branch_point_id, None)?;

    println!(
        "✅ Created branch '{}' with session ID: {} and timeline ID: {}",
        name, new_session_id, timeline_branch_id
    );

    Ok(())
}

async fn list_branches(session_id: &str) -> Result<(), TimeLoopError> {
    let branch_manager = timeloop_terminal::branch::BranchManager::new()?;
    let branches = branch_manager.get_branches_for_session(session_id)?;
    println!("Branches for session {}:", session_id);
    for b in branches {
        println!("- {} (id: {})", b.name, b.id);
    }
    Ok(())
}

async fn merge_branch(branch_id: &str, target_session_id: &str) -> Result<(), TimeLoopError> {
    let mut branch_manager = timeloop_terminal::branch::BranchManager::new()?;
    branch_manager.merge_branch(branch_id, target_session_id)?;
    println!(
        "Merged branch {} into session {}",
        branch_id, target_session_id
    );
    Ok(())
}

async fn delete_branch(branch_id: &str) -> Result<(), TimeLoopError> {
    let mut branch_manager = timeloop_terminal::branch::BranchManager::new()?;
    branch_manager.delete_branch(branch_id)?;
    println!("Deleted branch {}", branch_id);
    Ok(())
}

async fn show_summary(session_id: &str) -> Result<(), TimeLoopError> {
    info!("📊 Showing summary for session: {}", session_id);

    let session_manager = SessionManager::new()?;
    let summary = session_manager.get_session_summary(session_id)?;

    println!("📈 Session Summary for: {}", session_id);
    println!("{}", "─".repeat(50));
    println!("⏱️  Duration: {}", summary.duration);
    println!("⌨️  Commands executed: {}", summary.commands_executed);
    println!("📁 Files modified: {}", summary.files_modified);
    println!("🎯 Last command: {}", summary.last_command);

    Ok(())
}

async fn export_session(session_id: &str, output: &str) -> Result<(), TimeLoopError> {
    let storage = Storage::new()?;
    storage.export_session_to_file(session_id, output)?;
    println!("Exported session {} to {}", session_id, output);
    Ok(())
}

async fn show_session_tree() -> Result<(), TimeLoopError> {
    let session_manager = SessionManager::new()?;
    let tree = session_manager.get_session_tree()?;

    fn print_node(node: &timeloop_terminal::session::SessionNode, depth: usize) {
        let indent = "  ".repeat(depth);
        println!("{}- {} ({})", indent, node.session.name, node.session.id);
        for child in &node.children {
            print_node(child, depth + 1);
        }
    }

    println!("Session Tree:");
    for node in &tree {
        print_node(node, 0);
    }
    Ok(())
}

async fn show_event_timeline(session_id: &str) -> Result<(), TimeLoopError> {
    let storage = Storage::new()?;
    let mut events = storage.get_events_for_session(session_id)?;
    events.sort_by_key(|e| e.sequence_number);
    println!("Event timeline for session {}:", session_id);
    for e in events {
        println!(
            "{} [{}] seq={}",
            e.timestamp.to_rfc3339(),
            match &e.event_type {
                timeloop_terminal::EventType::KeyPress { key, .. } => format!("KeyPress {}", key),
                timeloop_terminal::EventType::Command { command, .. } =>
                    format!("Command {}", command),
                timeloop_terminal::EventType::FileChange {
                    path, change_type, ..
                } => format!("FileChange {:?} {}", change_type, path),
                timeloop_terminal::EventType::TerminalState { .. } => "TerminalState".to_string(),
                timeloop_terminal::EventType::SessionMetadata { name, .. } =>
                    format!("SessionMetadata {}", name),
            },
            e.sequence_number
        );
    }
    Ok(())
}

async fn import_session(input: &str) -> Result<(), TimeLoopError> {
    let storage = Storage::new()?;
    let id = storage.import_session_from_file(input)?;
    println!("Imported session with ID {}", id);
    Ok(())
}

#[cfg(feature = "ai")]
async fn run_ai_summarize(session_id: &str, model: Option<&str>) -> Result<(), TimeLoopError> {
    let model = model.unwrap_or("openrouter/auto");
    let summary = timeloop_terminal::ai::summarize_session(session_id, model).await?;
    println!("{}", summary);
    Ok(())
}

#[cfg(not(feature = "ai"))]
async fn run_ai_summarize(_session_id: &str, _model: Option<&str>) -> Result<(), TimeLoopError> {
    println!("AI feature not enabled. Rebuild with: cargo run --features ai");
    Ok(())
}
