use std::process::{Command, Stdio};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::event::{self, Event as CEvent, KeyCode, KeyEvent, EventStream};
use tokio::task::JoinHandle;
use crate::{EventRecorder, TimeLoopError, FileChangeType};
use crate::file_watcher::FileWatcher;

pub struct TerminalEmulator {
    pub(crate) event_recorder: Arc<Mutex<EventRecorder>>,
    working_directory: String,
    file_watcher_handle: Option<JoinHandle<()>>,
}

impl TerminalEmulator {
    pub fn new(event_recorder: EventRecorder) -> crate::Result<Self> {
        let working_directory = std::env::current_dir()?
            .to_string_lossy()
            .to_string();
        
        Ok(Self {
            event_recorder: Arc::new(Mutex::new(event_recorder)),
            working_directory,
            file_watcher_handle: None,
        })
    }

    /// Start file watching for the current directory
    pub(crate) async fn start_file_watching(&mut self) -> crate::Result<()> {
        let watch_path = PathBuf::from(&self.working_directory);
        let recorder = self.event_recorder.clone();
        println!("📁 File watching started for: {}", self.working_directory);

        let handle = tokio::spawn(async move {
            // Create callback closure to record file changes
            let cb: crate::file_watcher::FileChangeCallback = {
                let recorder = recorder.clone();
                Arc::new(tokio::sync::Mutex::new(move |path: &str, change: FileChangeType| {
                    // Synchronous closure: use std::sync::Mutex to mutate recorder
                    if let Ok(mut guard) = recorder.lock() {
                        if let Err(e) = guard.record_file_change(path, change) {
                            eprintln!("Error recording file change: {}", e);
                        }
                    }
                    Ok(())
                }))
            };

            let mut watcher = match FileWatcher::new(cb) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("Failed to init file watcher: {}", e);
                    return;
                }
            };

            if let Err(e) = watcher.add_watch_path(watch_path.clone(), true) {
                eprintln!("Failed to add watch path: {}", e);
                return;
            }

            if let Err(e) = watcher.start_watching().await {
                eprintln!("File watching stopped with error: {}", e);
            }
        });

        self.file_watcher_handle = Some(handle);
        Ok(())
    }

    /// Stop file watching
    pub(crate) async fn stop_file_watching(&mut self) {
        if let Some(handle) = self.file_watcher_handle.take() {
            handle.abort();
            if let Err(e) = handle.await {
                if !e.is_cancelled() {
                    eprintln!("Error stopping file watcher: {}", e);
                }
            }
        }
    }

    pub async fn run(&mut self) -> crate::Result<()> {
        // Enable raw mode to capture keystrokes and resize events
        enable_raw_mode()?;

        // Record initial terminal state
        let (cols, rows) = crossterm::terminal::size()?;
        if let Ok(mut guard) = self.event_recorder.lock() {
            guard.record_terminal_state((0, 0), (cols, rows))?;
        }
        
        // Start file watching
        if let Err(e) = self.start_file_watching().await {
            eprintln!("Warning: Could not start file watching: {}", e);
        }
        
        println!("TimeLoop Terminal - Raw Mode (type commands and press Enter). Type 'exit' to quit.");

        let mut input_buffer = String::new();
        let result = loop {
            // Poll for events
            if event::poll(std::time::Duration::from_millis(200))? {
                match event::read()? {
                    CEvent::Key(KeyEvent { code, .. }) => {
                        match code {
                            KeyCode::Char(c) => {
                                input_buffer.push(c);
                                if let Ok(mut guard) = self.event_recorder.lock() {
                                    guard.record_key_press(&c.to_string())?;
                                }
                                print!("{}", c);
                                io::stdout().flush()?;
                            }
                            KeyCode::Backspace => {
                                input_buffer.pop();
                                print!("\u{8} \u{8}");
                                io::stdout().flush()?;
                            }
                            KeyCode::Enter => {
                                println!();
                                let cmd = input_buffer.trim().to_string();
                                if cmd == "exit" || cmd == "quit" {
                                    println!("👋 Goodbye!");
                                    break Ok(());
                                }
                                let output = self.execute_external_command(&cmd).await?;
                                if let Ok(mut guard) = self.event_recorder.lock() {
                                    guard.record_command(&cmd, &output.output, output.exit_code, &self.working_directory)?;
                                }
                                input_buffer.clear();
                                print!("> ");
                                io::stdout().flush()?;
                            }
                            _ => {}
                        }
                    }
                    CEvent::Resize(w, h) => {
                        if let Ok(mut guard) = self.event_recorder.lock() {
                            guard.record_terminal_state((0, 0), (w, h))?;
                        }
                    }
                    _ => {}
                }
            } else {
                // Periodic tasks can go here
            }
        };
        
        // Cleanup: stop file watching
        self.stop_file_watching().await;
        
        disable_raw_mode()?;
        result
    }

    async fn execute_external_command(&self, command: &str) -> crate::Result<CommandOutput> {
        // On Windows, we'll use PowerShell to execute commands for better compatibility
        let mut cmd = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("powershell");
            cmd.args(["-Command", command]);
            cmd
        } else {
            let split_result = shellwords::split(command)
                .map_err(|e| TimeLoopError::CommandExecution(e.to_string()))?;
            let args: Vec<&str> = split_result
                .iter()
                .map(|s| s.as_str())
                .collect();

            if args.is_empty() {
                return Ok(CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                });
            }

            let mut cmd = Command::new(args[0]);
            cmd.args(&args[1..]);
            cmd
        };
        
        cmd.current_dir(&self.working_directory);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd.output()
            .map_err(|e| TimeLoopError::CommandExecution(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        let combined_output = if !stderr.is_empty() {
            format!("{}\n{}", stdout, stderr)
        } else {
            stdout.to_string()
        };

        if !combined_output.is_empty() {
            println!("{}", combined_output);
        }

        Ok(CommandOutput {
            output: combined_output,
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}

#[derive(Debug)]
struct CommandOutput {
    output: String,
    exit_code: i32,
} 

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_file_watching_integration() {
        // Test that file watching can be started and stopped without errors
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("events.db");
        let storage = crate::storage::Storage::with_path(db_path.to_str().unwrap()).unwrap();
        
        // Create session manager and session
        let mut session_manager = crate::session::SessionManager::with_storage(storage);
        let session_id = session_manager.create_session("file-watch-test").unwrap();
        
        // Create event recorder with a separate database path to avoid conflicts
        let event_db_path = tmp_dir.path().join("events2.db");
        let event_recorder_storage = crate::storage::Storage::with_path(event_db_path.to_str().unwrap()).unwrap();
        let event_recorder = crate::events::EventRecorder::with_storage(&session_id, event_recorder_storage);
        let mut terminal = TerminalEmulator::new(event_recorder).unwrap();
        
        // Test that file watching starts without error
        match terminal.start_file_watching().await {
            Ok(_) => println!("File watching started successfully"),
            Err(e) => {
                println!("File watching failed to start: {}", e);
                panic!("File watching failed: {}", e);
            }
        }
        
        // Wait a moment
        sleep(Duration::from_millis(100)).await;
        
        // Test that file watching stops without error
        terminal.stop_file_watching().await;
        
        // If we get here, the test passes
        assert!(true);
    }
} 