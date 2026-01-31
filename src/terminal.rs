use crate::file_watcher::FileWatcher;
use crate::{EventRecorder, FileChangeType, TimeLoopError};
use crossterm::{
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{disable_raw_mode, enable_raw_mode},
    ExecutableCommand,
};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::process::Command;
use tokio::task::JoinHandle;

pub struct TerminalEmulator {
    pub(crate) event_recorder: Arc<Mutex<EventRecorder>>,
    working_directory: String,
    file_watcher_handle: Option<JoinHandle<()>>,
    // Command history with a maximum size
    command_history: VecDeque<String>,
}

impl TerminalEmulator {
    pub fn new(event_recorder: EventRecorder) -> crate::Result<Self> {
        let working_directory = std::env::current_dir()?.to_string_lossy().to_string();

        Ok(Self {
            event_recorder: Arc::new(Mutex::new(event_recorder)),
            working_directory,
            file_watcher_handle: None,
            command_history: VecDeque::with_capacity(100), // Store up to 100 commands
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
                Arc::new(tokio::sync::Mutex::new(
                    move |path: &str, change: FileChangeType| {
                        // Synchronous closure: use std::sync::Mutex to mutate recorder
                        if let Ok(mut guard) = recorder.lock() {
                            if let Err(e) = guard.record_file_change(path, change) {
                                eprintln!("Error recording file change: {}", e);
                            }
                        }
                        Ok(())
                    },
                ))
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

        // Print welcome message with styling
        let mut stdout = io::stdout();
        stdout.execute(SetForegroundColor(Color::Cyan))?;
        println!("╔════════════════════════════════════════════════════╗");
        println!("║                                                    ║");
        stdout.execute(SetForegroundColor(Color::Blue))?;

        if cfg!(target_os = "windows") {
            println!("║            TimeLoop Terminal - PowerShell          ║");
        } else {
            println!("║              TimeLoop Terminal - Bash              ║");
        }

        stdout.execute(SetForegroundColor(Color::Cyan))?;
        println!("║                                                    ║");
        println!("╚════════════════════════════════════════════════════╝");
        stdout.execute(ResetColor)?;

        // Print help info
        stdout.execute(SetForegroundColor(Color::Yellow))?;
        println!("Type 'exit' to quit | All shell commands are supported");
        stdout.execute(ResetColor)?;
        println!("─────────────────────────────────────────────────────");

        // Main loop using standard input
        let stdin = io::stdin();
        let mut stdout = io::stdout();

        let result = loop {
            // Check incognito status for prompt
            let is_incognito = if let Ok(guard) = self.event_recorder.lock() {
                guard.is_paused()
            } else {
                false
            };

            // Display styled prompt
            if is_incognito {
                stdout.execute(SetForegroundColor(Color::Magenta))?;
                print!("🕵️ ");
            } else {
                stdout.execute(SetForegroundColor(Color::Green))?;
                print!("⚡ ");
            }
            stdout.execute(SetForegroundColor(Color::Blue))?;
            print!("[{}]", self.working_directory);
            stdout.execute(SetForegroundColor(Color::Yellow))?;
            print!(" > ");
            stdout.execute(ResetColor)?;
            stdout.flush()?;

            // Read a line of input
            let mut input = String::new();
            stdin.read_line(&mut input)?;

            // Trim the input
            let input = input.trim();

            // Record the command
            if let Ok(mut guard) = self.event_recorder.lock() {
                for c in input.chars() {
                    guard.record_key_press(&c.to_string())?;
                }
            }

            // Skip empty input
            if input.is_empty() {
                continue;
            }

            // Add command to history if not empty
            if !input.is_empty() {
                // Add to history, removing oldest if at capacity
                if self.command_history.len() >= 100 {
                    self.command_history.pop_front();
                }
                self.command_history.push_back(input.to_string());
            }

            // Handle internal commands
            if input == "exit" || input == "quit" {
                stdout.execute(SetForegroundColor(Color::Green))?;
                println!("👋 Goodbye!");
                stdout.execute(ResetColor)?;
                break Ok(());
            } else if input == "incognito" {
                let is_paused = if let Ok(mut guard) = self.event_recorder.lock() {
                    if guard.is_paused() {
                        guard.resume_recording();
                        false
                    } else {
                        guard.pause_recording();
                        true
                    }
                } else {
                    false
                };

                stdout.execute(SetForegroundColor(Color::Magenta))?;
                if is_paused {
                    println!("🕵️ Incognito mode enabled. Recording paused.");
                } else {
                    println!("📝 Incognito mode disabled. Recording resumed.");
                }
                stdout.execute(ResetColor)?;
                continue;
            } else {
                // Special handling for directory change commands
                if input == "cd" || input == "cd ~" {
                    // Change to home directory
                    let home = std::env::var("USERPROFILE")
                        .or_else(|_| std::env::var("HOME"))
                        .unwrap_or_else(|_| ".".to_string());

                    if let Err(e) = std::env::set_current_dir(&home) {
                        println!("Error changing to home directory: {}", e);
                    } else {
                        // Update working directory if successful
                        if let Ok(new_dir) = std::env::current_dir() {
                            self.working_directory = new_dir.to_string_lossy().to_string();
                        }
                    }
                } else if input == "cd .." {
                    // Go up one directory
                    if let Err(e) = std::env::set_current_dir("..") {
                        println!("Error going up directory: {}", e);
                    } else {
                        // Update working directory if successful
                        if let Ok(new_dir) = std::env::current_dir() {
                            self.working_directory = new_dir.to_string_lossy().to_string();
                        }
                    }
                } else if input.starts_with("cd ") {
                    // Extract the directory path
                    let path = input.trim_start_matches("cd ").trim();

                    // Try to change directory
                    if let Err(e) = std::env::set_current_dir(path) {
                        println!("Error changing directory: {}", e);
                    } else {
                        // Update working directory if successful
                        if let Ok(new_dir) = std::env::current_dir() {
                            self.working_directory = new_dir.to_string_lossy().to_string();
                        }
                    }
                } else if input.starts_with("Set-Location ")
                    || input.starts_with("sl ")
                    || input.starts_with("chdir ")
                {
                    // Extract the directory path from the PowerShell command
                    let path_start = input.find(' ').map(|i| i + 1).unwrap_or(input.len());
                    let path = input[path_start..].trim();

                    // Remove quotes if present
                    let path = path.trim_start_matches('"').trim_end_matches('"');

                    // Try to change directory directly
                    if let Err(e) = std::env::set_current_dir(path) {
                        // If direct change fails, execute via PowerShell and show output
                        let output = self.execute_external_command(input).await?;
                        if let Ok(mut guard) = self.event_recorder.lock() {
                            guard.record_command(
                                input,
                                &output.output,
                                output.exit_code,
                                &self.working_directory,
                            )?;
                        }
                        println!("Error changing directory: {}", e);
                    } else {
                        // Record the command but don't execute it again
                        if let Ok(mut guard) = self.event_recorder.lock() {
                            guard.record_command(input, "", 0, &self.working_directory)?;
                        }

                        // Update working directory
                        if let Ok(new_dir) = std::env::current_dir() {
                            self.working_directory = new_dir.to_string_lossy().to_string();
                        }
                    }
                } else {
                    // For all other commands, just execute them normally
                    let output = self.execute_external_command(input).await?;
                    if let Ok(mut guard) = self.event_recorder.lock() {
                        guard.record_command(
                            input,
                            &output.output,
                            output.exit_code,
                            &self.working_directory,
                        )?;
                    }
                }
            }
        };

        // Cleanup: stop file watching
        self.stop_file_watching().await;

        disable_raw_mode()?;
        result
    }

    async fn execute_external_command(&self, command: &str) -> crate::Result<CommandOutput> {
        // Use the appropriate shell based on the platform
        let mut cmd = if cfg!(target_os = "windows") {
            // On Windows, use PowerShell with proper arguments to execute commands
            let mut cmd = Command::new("powershell");
            // Use -NoProfile to start faster, -ExecutionPolicy Bypass to allow script execution
            cmd.args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                command,
            ]);
            cmd
        } else {
            // On Unix systems, use bash with -c to execute commands
            let mut cmd = Command::new("bash");
            cmd.args(["-c", command]);
            cmd
        };

        cmd.current_dir(&self.working_directory);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
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
        let event_recorder_storage =
            crate::storage::Storage::with_path(event_db_path.to_str().unwrap()).unwrap();
        let event_recorder =
            crate::events::EventRecorder::with_storage(&session_id, event_recorder_storage);
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

    #[tokio::test(flavor = "current_thread")]
    async fn test_blocking_behavior() {
        // Setup
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("events_bench.db");
        let storage = crate::storage::Storage::with_path(db_path.to_str().unwrap()).unwrap();
        let mut session_manager = crate::session::SessionManager::with_storage(storage);
        let session_id = session_manager.create_session("bench-test").unwrap();

        let event_db_path = tmp_dir.path().join("events_bench_rec.db");
        let event_storage = crate::storage::Storage::with_path(event_db_path.to_str().unwrap()).unwrap();
        let event_recorder = crate::events::EventRecorder::with_storage(&session_id, event_storage);
        let terminal = TerminalEmulator::new(event_recorder).unwrap();

        // Shared counter
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = counter.clone();

        // Start background task
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(20)).await;
        let start_count = counter.load(std::sync::atomic::Ordering::Relaxed);

        // Run sleep command for 1 second
        let cmd = if cfg!(target_os = "windows") { "Start-Sleep -Seconds 1" } else { "sleep 1" };

        let _ = terminal.execute_external_command(cmd).await.unwrap();

        let end_count = counter.load(std::sync::atomic::Ordering::Relaxed);
        let diff = end_count - start_count;

        println!("Counter diff: {}", diff);

        // Clean up background task
        handle.abort();

        assert!(diff > 50, "Expected non-blocking behavior, but counter only increased by {}", diff);
    }
}
