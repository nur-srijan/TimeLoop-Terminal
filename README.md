# 🔮 TimeLoop Terminal

> **Your terminal is not just a tool. It's a time machine.**

A superintelligent terminal emulator that records every keystroke, file change, CLI command, and shell state, allowing you to rewind, replay, and branch your coding sessions like a Git repository for your terminal history.

## Mermaid Diagram

flowchart TD
    CLI["CLI Layer"]:::api
    API["Public Library API"]:::api
    Terminal["Terminal Emulator"]:::infra
    FileWatcher["File Watcher"]:::infra
    Recorder["Event Recorder"]:::core
    Session["Session Lifecycle"]:::core
    Storage["Storage Engine"]:::infra
    Replay["Replay Engine"]:::core
    Branch["Branch Manager"]:::core
    Error["Error Handler"]:::error
    ExternalFS["Local File System"]:::external
    Crossterm["crossterm"]:::external
    SledDB["sled DB"]:::external
    UserShell["User Shell"]:::external

    %% CLI to API
    CLI -->|"commands (start/stop/list/replay/branch)"| API

    %% API routes to domain components
    API --> Recorder
    API --> Session
    API --> Replay
    API --> Branch

    %% User input flow
    UserShell -->|"keystroke/input"| Terminal
    Terminal -->|"crossterm calls"| Crossterm
    Terminal -->|"emit events"| Recorder

    %% File watcher flow
    ExternalFS --> FileWatcher
    FileWatcher -->|"file change events"| Recorder

    %% Recording to storage
    Recorder -->|"append events"| Storage
    Session -->|"session metadata"| Storage

    %% Replay flow
    Replay -->|"read events"| Storage
    Replay -->|"render playback"| Terminal

    %% Branch management
    Branch -->|"read/write branch metadata"| Storage
    Storage -->|"branch data"| Branch

    %% Error handling cross-cutting
    Recorder -->|"errors"| Error
    Session -->|"errors"| Error
    Terminal -->|"errors"| Error
    FileWatcher -->|"errors"| Error
    Storage -->|"errors"| Error
    Replay -->|"errors"| Error
    Branch -->|"errors"| Error
    API -->|"errors"| Error
    CLI -->|"errors"| Error

    %% External DB integration
    Storage --> SledDB

    %% Legend grouping (not clickable)
    classDef core fill:#fbb,stroke:#333,stroke-width:1px
    classDef infra fill:#bbf,stroke:#333,stroke-width:1px
    classDef api fill:#bfb,stroke:#333,stroke-width:1px
    classDef external fill:#ccc,stroke:#333,stroke-width:1px
    classDef error fill:#f99,stroke:#333,stroke-width:1px

    %% Click events
    click CLI "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/main.rs"
    click API "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/lib.rs"
    click Error "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/error.rs"
    click Recorder "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/events.rs"
    click Session "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/session.rs"
    click FileWatcher "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/file_watcher.rs"
    click Storage "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/storage.rs"
    click Replay "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/replay.rs"
    click Branch "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/branch.rs"
    click Terminal "https://github.com/nur-srijan/timeloop-terminal/blob/main/src/terminal.rs"

## ✨ Features

### 🎬 Session Recording
- **Complete Capture**: Records every keystroke, command, output, and file change
- **Real-time Monitoring**: Watches file system changes and terminal state
- **Persistent Storage**: Uses event sourcing with sled database for reliable storage

### ⏮ Rewind & Replay
- **YouTube-style Controls**: Rewind your terminal like a video
- **Variable Speed**: Replay at 0.5x, 1x, 2x, or 5x speed
- **Visual Playback**: See your commands and outputs replayed with timestamps
- **Range Replay**: Replay specific time ranges within a session

### 🧬 Timeline Branching
- **Git-style Branches**: Create alternate futures from any past command
- **Branch Points**: Choose exactly where to diverge from your timeline
- **Merge Capabilities**: Merge branches back into main timeline
- **Branch Management**: List, view, and manage your timeline branches

### 📊 Session Analytics
- **Activity Summary**: See commands executed, files modified, duration
- **Session Tree**: Visualize session relationships and branches
- **Event Timeline**: Browse all recorded events chronologically

## 🚀 Quick Start

### Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/timeloop-terminal.git
cd timeloop-terminal

# Build the project
cargo build --release

# Run the terminal
cargo run
```

### Basic Usage

```bash
# Start a new session
timeloop start --name "my-coding-session"

# List all sessions
timeloop list

# Replay a session at 2x speed
timeloop replay <session-id> --speed 2.0

# Create a branch from a session
timeloop branch <session-id> "experimental-feature"

# Show session summary
timeloop summary <session-id>
```

## 🎯 Use Cases

### 🎓 Learning & Tutorials
- Record your coding sessions for later review
- Create step-by-step tutorials with perfect replay
- Share your workflow with others

### 🐛 Debugging
- Replay the exact sequence that led to a bug
- Branch from before the bug and try different approaches
- Compare different debugging strategies

### 🔄 Experimentation
- Try risky commands in a branch
- Compare different approaches to the same problem
- Never lose your original work

### 👥 Collaboration
- Share your terminal sessions with teammates
- Review each other's workflows
- Pair program across branched timelines

## 🛠️ Architecture

### Core Components

- **Event Recorder**: Captures all terminal interactions
- **Storage Engine**: Persistent event storage using sled
- **Replay Engine**: Handles session playback and visualization
- **Branch Manager**: Manages timeline branching and merging
- **File Watcher**: Monitors file system changes
- **Terminal Emulator**: Provides the interactive terminal interface

### Event Types

- **KeyPress**: Individual keystrokes and navigation
- **Command**: Executed commands with output and exit codes
- **FileChange**: File creation, modification, deletion, and renaming
- **TerminalState**: Cursor position and screen size changes
- **SessionMetadata**: Session creation and configuration

## 📁 Project Structure

```
src/
├── main.rs          # CLI entry point
├── lib.rs           # Library exports
├── error.rs         # Error handling
├── events.rs        # Event recording and types
├── session.rs       # Session management
├── terminal.rs      # Terminal emulator
├── replay.rs        # Session replay engine
├── branch.rs        # Timeline branching
├── storage.rs       # Persistent storage
└── file_watcher.rs  # File system monitoring
```

## 🔧 Configuration

### File Watching

The terminal automatically watches your current directory for file changes. You can configure ignored patterns:

```rust
// Default ignored patterns
.git
target
node_modules
.DS_Store
*.tmp
*.log
```

### Storage Location

Sessions are stored in your system's local data directory:
- **Windows**: `%LOCALAPPDATA%/timeloop-terminal/`
- **macOS**: `~/Library/Application Support/timeloop-terminal/`
- **Linux**: `~/.local/share/timeloop-terminal/`

## 🎨 Advanced Features

### Custom Commands

```bash
# Replay a specific time range
timeloop replay-range <session-id> --start "2024-01-15T10:00:00Z" --end "2024-01-15T11:00:00Z"

# Export session to JSON
timeloop export <session-id> --format json

# Import session from backup
timeloop import <backup-file>
```

### Branch Operations

```bash
# List all branches for a session
timeloop branches <session-id>

# Merge a branch into current session
timeloop merge <branch-id>

# Delete a branch
timeloop delete-branch <branch-id>
```

## 🔮 Future Features

### 🧠 AI Integration
- **Auto-summarize**: AI-generated summaries of your sessions
- **Command Suggestions**: Intelligent command recommendations
- **Pattern Recognition**: Identify common workflows and suggest optimizations

### 🎙️ Natural Language Queries
- "Show me what I did after 3 PM yesterday that broke my Python script"
- "Replay the session where I was debugging the authentication issue"
- "Create a branch from when I was working on the user interface"

### 🕹️ Enhanced UI
- **TUI Interface**: Rich terminal user interface for session management
- **Visual Timeline**: Graphical representation of your session history
- **Diff Viewer**: Side-by-side comparison of file changes

### 🧑‍🚀 Multiplayer Features
- **Shared Sessions**: Collaborate on the same terminal timeline
- **Real-time Sync**: Multiple users working in the same session
- **Conflict Resolution**: Handle conflicting changes in shared sessions

## 🤝 Contributing

We welcome contributions! Here's how you can help:

1. **Fork** the repository
2. **Create** a feature branch: `git checkout -b feature/amazing-feature`
3. **Commit** your changes: `git commit -m 'Add amazing feature'`
4. **Push** to the branch: `git push origin feature/amazing-feature`
5. **Open** a Pull Request

### Development Setup

```bash
# Install dependencies
cargo build

# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run

# Format code
cargo fmt

# Check for issues
cargo clippy
```

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by the concept of "time travel debugging"
- Built with Rust for performance and reliability
- Uses sled for embedded database storage
- Leverages crossterm for cross-platform terminal handling

## 🐛 Known Issues

- File watching may be limited on some systems
- Large sessions may consume significant storage
- Some terminal applications may not work perfectly in the emulator

## 📞 Support

- **Issues**: [GitHub Issues](https://github.com/yourusername/timeloop-terminal/issues)
- **Discussions**: [GitHub Discussions](https://github.com/yourusername/timeloop-terminal/discussions)
- **Documentation**: [Wiki](https://github.com/yourusername/timeloop-terminal/wiki)

---

**TimeLoop Terminal** - Because your terminal history deserves to be more than just a list of commands. 🕰️✨ 
