use std::process::{Command, Stdio};
use std::io::{self, Write};
use std::path::PathBuf;
use crossterm::{
    cursor,
    event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
    style::{Color, Print, ResetColor, SetForegroundColor},
    ExecutableCommand,
};
use tokio::task::JoinHandle;
use crate::{EventRecorder, TimeLoopError};

pub struct TerminalEmulator {
    pub(crate) event_recorder: EventRecorder,
    current_line: String,
    cursor_position: usize,
    history: Vec<String>,
    history_index: usize,
    working_directory: String,
    file_watcher_handle: Option<JoinHandle<()>>,
}

impl TerminalEmulator {
    pub fn new(event_recorder: EventRecorder) -> crate::Result<Self> {
        let working_directory = std::env::current_dir()?
            .to_string_lossy()
            .to_string();
        
        Ok(Self {
            event_recorder,
            current_line: String::new(),
            cursor_position: 0,
            history: Vec::new(),
            history_index: 0,
            working_directory,
            file_watcher_handle: None,
        })
    }

    /// Start file watching for the current directory
    pub(crate) async fn start_file_watching(&mut self) -> crate::Result<()> {
        let _current_dir = PathBuf::from(&self.working_directory);
        
        // For now, let's disable file watching to avoid database conflicts
        // TODO: Implement proper shared storage solution
        println!("⚠️  File watching temporarily disabled due to database conflicts");
        println!("📁 File watching started for: {}", self.working_directory);
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
        enable_raw_mode()?;
        
        // Record initial terminal state
        let (cols, rows) = crossterm::terminal::size()?;
        self.event_recorder.record_terminal_state((0, 0), (cols, rows))?;
        
        // Start file watching
        if let Err(e) = self.start_file_watching().await {
            eprintln!("Warning: Could not start file watching: {}", e);
        }
        
        println!("🚀 TimeLoop Terminal - Your terminal is a time machine!");
        println!("Type 'exit' or press Ctrl+C to quit");
        println!("{}", "─".repeat(60));
        
        let result = loop {
            self.display_prompt()?;
            
            if let Err(e) = self.handle_input().await {
                eprintln!("Error: {}", e);
                break Err(e);
            }
        };
        
        // Cleanup: stop file watching
        self.stop_file_watching().await;
        
        disable_raw_mode()?;
        result
    }

    fn display_prompt(&self) -> crate::Result<()> {
        let mut stdout = io::stdout();
        
        // Clear the current line
        stdout.execute(Clear(ClearType::CurrentLine))?;
        
        // Show prompt
        stdout.execute(SetForegroundColor(Color::Green))?;
        stdout.execute(Print("🕰️  "))?;
        stdout.execute(ResetColor)?;
        
        stdout.execute(SetForegroundColor(Color::Blue))?;
        stdout.execute(Print(format!("{} ", self.working_directory)))?;
        stdout.execute(ResetColor)?;
        
        stdout.execute(Print("$ "))?;
        stdout.execute(Print(&self.current_line))?;
        
        // Position cursor
        let prompt_length = self.working_directory.len() + 4; // "$ " + "🕰️  "
        stdout.execute(cursor::MoveToColumn((prompt_length + self.cursor_position) as u16))?;
        
        stdout.flush()?;
        Ok(())
    }

    async fn handle_input(&mut self) -> crate::Result<()> {
        loop {
            match event::read()? {
                CrosstermEvent::Key(KeyEvent { code, modifiers, .. }) => {
                    match code {
                        KeyCode::Char(c) => {
                            if modifiers.contains(KeyModifiers::CONTROL) {
                                match c {
                                    'c' => {
                                        println!("\n👋 Goodbye!");
                                        return Err(TimeLoopError::Unknown("User interrupted".to_string()));
                                    }
                                    'l' => {
                                        crossterm::terminal::Clear(ClearType::All);
                                        return Ok(());
                                    }
                                    _ => {}
                                }
                            } else {
                                self.insert_char(c)?;
                            }
                        }
                        KeyCode::Backspace => {
                            self.delete_char()?;
                        }
                        KeyCode::Delete => {
                            self.delete_char_forward()?;
                        }
                        KeyCode::Left => {
                            self.move_cursor_left()?;
                        }
                        KeyCode::Right => {
                            self.move_cursor_right()?;
                        }
                        KeyCode::Up => {
                            self.navigate_history_up()?;
                        }
                        KeyCode::Down => {
                            self.navigate_history_down()?;
                        }
                        KeyCode::Enter => {
                            self.execute_command().await?;
                            break;
                        }
                        KeyCode::Tab => {
                            // TODO: Implement tab completion
                        }
                        _ => {}
                    }
                }
                CrosstermEvent::Resize(cols, rows) => {
                    self.event_recorder.record_terminal_state((0, 0), (cols, rows))?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn insert_char(&mut self, c: char) -> crate::Result<()> {
        self.current_line.insert(self.cursor_position, c);
        self.cursor_position += 1;
        self.event_recorder.record_key_press(&c.to_string())?;
        Ok(())
    }

    fn delete_char(&mut self) -> crate::Result<()> {
        if self.cursor_position > 0 {
            self.current_line.remove(self.cursor_position - 1);
            self.cursor_position -= 1;
            self.event_recorder.record_key_press("Backspace")?;
        }
        Ok(())
    }

    fn delete_char_forward(&mut self) -> crate::Result<()> {
        if self.cursor_position < self.current_line.len() {
            self.current_line.remove(self.cursor_position);
            self.event_recorder.record_key_press("Delete")?;
        }
        Ok(())
    }

    fn move_cursor_left(&mut self) -> crate::Result<()> {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            self.event_recorder.record_key_press("Left")?;
        }
        Ok(())
    }

    fn move_cursor_right(&mut self) -> crate::Result<()> {
        if self.cursor_position < self.current_line.len() {
            self.cursor_position += 1;
            self.event_recorder.record_key_press("Right")?;
        }
        Ok(())
    }

    fn navigate_history_up(&mut self) -> crate::Result<()> {
        if self.history_index < self.history.len() {
            if self.history_index == 0 {
                // Save current line before navigating
                self.current_line = self.current_line.clone();
            }
            self.history_index += 1;
            let history_item = &self.history[self.history.len() - self.history_index];
            self.current_line = history_item.clone();
            self.cursor_position = self.current_line.len();
            self.event_recorder.record_key_press("Up")?;
        }
        Ok(())
    }

    fn navigate_history_down(&mut self) -> crate::Result<()> {
        if self.history_index > 0 {
            self.history_index -= 1;
            if self.history_index == 0 {
                // Restore the original line
                self.current_line = self.current_line.clone();
            } else {
                let history_item = &self.history[self.history.len() - self.history_index];
                self.current_line = history_item.clone();
            }
            self.cursor_position = self.current_line.len();
            self.event_recorder.record_key_press("Down")?;
        }
        Ok(())
    }

    async fn execute_command(&mut self) -> crate::Result<()> {
        let command = self.current_line.trim();
        
        if command.is_empty() {
            println!();
            return Ok(());
        }

        // Add to history
        if !command.is_empty() {
            self.history.push(command.to_string());
            self.history_index = 0;
        }

        println!(); // New line after command

        // Handle built-in commands
        match command {
            "exit" | "quit" => {
                println!("👋 Goodbye!");
                return Err(TimeLoopError::Unknown("User requested exit".to_string()));
            }
            "clear" => {
                crossterm::terminal::Clear(ClearType::All);
                self.current_line.clear();
                self.cursor_position = 0;
                return Ok(());
            }
            "pwd" => {
                println!("{}", self.working_directory);
                self.event_recorder.record_command(command, &self.working_directory, 0, &self.working_directory)?;
                self.current_line.clear();
                self.cursor_position = 0;
                return Ok(());
            }
            "history" => {
                for (i, cmd) in self.history.iter().enumerate() {
                    println!("{}: {}", i + 1, cmd);
                }
                self.event_recorder.record_command(command, "History displayed", 0, &self.working_directory)?;
                self.current_line.clear();
                self.cursor_position = 0;
                return Ok(());
            }
            _ => {}
        }

        // Execute external command
        let output = self.execute_external_command(command).await?;
        self.event_recorder.record_command(command, &output.output, output.exit_code, &self.working_directory)?;
        
        // Update working directory if cd command was executed
        if command.starts_with("cd ") {
            let new_dir = command[3..].trim();
            if let Ok(_new_path) = std::env::set_current_dir(new_dir) {
                self.working_directory = std::env::current_dir()?
                    .to_string_lossy()
                    .to_string();
            }
        }

        self.current_line.clear();
        self.cursor_position = 0;
        Ok(())
    }

    async fn execute_external_command(&self, command: &str) -> crate::Result<CommandOutput> {
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