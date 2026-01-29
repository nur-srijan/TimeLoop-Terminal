use thiserror::Error;

#[derive(Error, Debug)]
pub enum TimeLoopError {
    #[error("Terminal error: {0}")]
    Terminal(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Invalid session ID: {0}")]
    InvalidSessionId(String),

    #[error("File system error: {0}")]
    FileSystem(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("CBOR serialization error: {0}")]
    SerializationCbor(#[from] serde_cbor::Error),

    #[error("Event recording error: {0}")]
    EventRecording(String),

    #[error("Replay error: {0}")]
    Replay(String),

    #[error("Branch error: {0}")]
    Branch(String),

    #[error("File watcher error: {0}")]
    FileWatcher(#[from] notify::Error),

    #[error("Command execution error: {0}")]
    CommandExecution(String),

    #[error("Configuration error: {0}")]
    Configuration(String),
    
    #[error("GPU rendering error: {0}")]
    GpuError(String),
    
    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl From<anyhow::Error> for TimeLoopError {
    fn from(err: anyhow::Error) -> Self {
        TimeLoopError::Unknown(err.to_string())
    }
}
