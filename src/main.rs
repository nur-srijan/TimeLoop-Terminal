use clap::{Parser, Subcommand};
use timeloop_terminal::{
    session::SessionManager,
    terminal::TerminalEmulator,
    events::EventRecorder,
    replay::ReplayEngine,
    error::TimeLoopError,
    storage::Storage,
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
    /// Create a branch from a session
    Branch {
        /// Source session ID
        session_id: String,
        /// Branch name
        name: String,
    },
    /// Show session summary
    Summary {
        /// Session ID
        session_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), TimeLoopError> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    info!("🚀 Starting TimeLoop Terminal...");
    
    let cli = Cli::parse();
    
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
        Some(Commands::Branch { session_id, name }) => {
            create_branch(session_id, name).await?;
        }
        Some(Commands::Summary { session_id }) => {
            show_summary(session_id).await?;
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
        println!("📁 {} - {} ({})", 
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

async fn create_branch(session_id: &str, name: &str) -> Result<(), TimeLoopError> {
    info!("🧬 Creating branch '{}' from session: {}", name, session_id);
    
    let mut session_manager = SessionManager::new()?;
    let branch_id = session_manager.create_branch(session_id, name)?;
    
    println!("✅ Created branch '{}' with ID: {}", name, branch_id);
    
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