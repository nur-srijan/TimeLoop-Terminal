use std::sync::Arc;
use std::collections::VecDeque;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode},
    style::{Color, SetForegroundColor, ResetColor},
    ExecutableCommand,
};
use tokio::task::JoinHandle;
use crate::{EventRecorder, TimeLoopError, FileChangeType, GpuRenderer};
use crate::file_watcher::FileWatcher;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
};

/// GPU-enabled terminal emulator that renders text using wgpu
pub struct GpuTerminalEmulator {
    pub(crate) event_recorder: Arc<std::sync::Mutex<EventRecorder>>,
    working_directory: String,
    file_watcher_handle: Option<JoinHandle<()>>,
    command_history: VecDeque<String>,
    gpu_renderer: Option<GpuRenderer>,
    terminal_buffer: Vec<String>,
    cursor_x: usize,
    cursor_y: usize,
    terminal_width: usize,
    terminal_height: usize,
}

impl GpuTerminalEmulator {
    /// Create a new GPU terminal emulator
    pub fn new(event_recorder: EventRecorder) -> crate::Result<Self> {
        let working_directory = std::env::current_dir()?
            .to_string_lossy()
            .to_string();
        
        Ok(Self {
            event_recorder: Arc::new(std::sync::Mutex::new(event_recorder)),
            working_directory,
            file_watcher_handle: None,
            command_history: VecDeque::with_capacity(100),
            gpu_renderer: None,
            terminal_buffer: vec![String::new()],
            cursor_x: 0,
            cursor_y: 0,
            terminal_width: 80,
            terminal_height: 24,
        })
    }
    
    /// Initialize the GPU renderer
    pub async fn init_gpu_renderer(&mut self) -> Result<(), TimeLoopError> {
        // This would be called from a GUI context
        // For now, we'll create a placeholder
        Ok(())
    }
    
    /// Start file watching for the current directory
    pub(crate) async fn start_file_watching(&mut self) -> crate::Result<()> {
        let watch_path = std::path::PathBuf::from(&self.working_directory);
        let recorder = self.event_recorder.clone();
        println!("📁 File watching started for: {}", self.working_directory);

        let handle = tokio::spawn(async move {
            let cb: crate::file_watcher::FileChangeCallback = {
                let recorder = recorder.clone();
                Arc::new(tokio::sync::Mutex::new(move |path: &str, change: FileChangeType| {
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
    
    /// Add text to the terminal buffer
    pub fn add_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => {
                    self.cursor_y += 1;
                    self.cursor_x = 0;
                    if self.cursor_y >= self.terminal_height {
                        self.terminal_buffer.remove(0);
                        self.cursor_y = self.terminal_height - 1;
                    }
                    if self.cursor_y >= self.terminal_buffer.len() {
                        self.terminal_buffer.push(String::new());
                    }
                }
                '\r' => {
                    self.cursor_x = 0;
                }
                '\t' => {
                    self.cursor_x = (self.cursor_x / 4 + 1) * 4;
                }
                _ => {
                    if self.cursor_x >= self.terminal_width {
                        self.cursor_x = 0;
                        self.cursor_y += 1;
                        if self.cursor_y >= self.terminal_height {
                            self.terminal_buffer.remove(0);
                            self.cursor_y = self.terminal_height - 1;
                        }
                        if self.cursor_y >= self.terminal_buffer.len() {
                            self.terminal_buffer.push(String::new());
                        }
                    }
                    
                    if self.cursor_y < self.terminal_buffer.len() {
                        let line = &mut self.terminal_buffer[self.cursor_y];
                        if self.cursor_x >= line.len() {
                            line.push_str(&" ".repeat(self.cursor_x - line.len()));
                        }
                        line.insert(self.cursor_x, ch);
                        self.cursor_x += 1;
                    }
                }
            }
        }
    }
    
    /// Get the current terminal content as a single string
    pub fn get_terminal_content(&self) -> String {
        self.terminal_buffer.join("\n")
    }
    
    /// Render the terminal using GPU
    pub fn render_gpu(&mut self, time: f32) -> Result<(), TimeLoopError> {
        if let Some(ref mut renderer) = self.gpu_renderer {
            let content = self.get_terminal_content();
            renderer.render(&content, time)?;
        }
        Ok(())
    }
    
    /// Run the GPU terminal (this would typically be called from a GUI context)
    pub async fn run_gpu(&mut self) -> crate::Result<()> {
        // Enable raw mode
        enable_raw_mode()?;
        
        // Record initial terminal state
        let (cols, rows) = crossterm::terminal::size()?;
        self.terminal_width = cols as usize;
        self.terminal_height = rows as usize;
        
        if let Ok(mut guard) = self.event_recorder.lock() {
            guard.record_terminal_state((0, 0), (cols, rows))?;
        }
        
        // Start file watching
        if let Err(e) = self.start_file_watching().await {
            eprintln!("Warning: Could not start file watching: {}", e);
        }
        
        // Print welcome message
        let mut stdout = std::io::stdout();
        stdout.execute(SetForegroundColor(Color::Cyan))?;
        println!("╔════════════════════════════════════════════════════╗");
        println!("║                                                    ║");
        stdout.execute(SetForegroundColor(Color::Blue))?;
        println!("║            TimeLoop Terminal - GPU Mode            ║");
        stdout.execute(SetForegroundColor(Color::Cyan))?;
        println!("║                                                    ║");
        println!("╚════════════════════════════════════════════════════╝");
        stdout.execute(ResetColor)?;
        
        // Add welcome message to buffer
        self.add_text("TimeLoop Terminal - GPU Mode\n");
        self.add_text("Type 'exit' to quit | All shell commands are supported\n");
        self.add_text("─────────────────────────────────────────────────────\n");
        
        // Main loop using standard input
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        
        let result = loop {
            // Display styled prompt
            stdout.execute(SetForegroundColor(Color::Green))?;
            print!("⚡ ");
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
            
            // Add to terminal buffer
            self.add_text(&format!("{} > {}\n", self.working_directory, input));
            
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
                if self.command_history.len() >= 100 {
                    self.command_history.pop_front();
                }
                self.command_history.push_back(input.to_string());
            }
            
            // Handle exit command
            if input == "exit" || input == "quit" {
                stdout.execute(SetForegroundColor(Color::Green))?;
                println!("👋 Goodbye!");
                stdout.execute(ResetColor)?;
                self.add_text("👋 Goodbye!\n");
                break Ok(());
            } else {
                // Execute command and add output to buffer
                let output = self.execute_external_command(input).await?;
                if let Ok(mut guard) = self.event_recorder.lock() {
                    guard.record_command(input, &output.output, output.exit_code, &self.working_directory)?;
                }
                
                if !output.output.is_empty() {
                    self.add_text(&output.output);
                    self.add_text("\n");
                }
            }
        };
        
        // Cleanup
        self.stop_file_watching().await;
        disable_raw_mode()?;
        result
    }
    
    async fn execute_external_command(&self, command: &str) -> crate::Result<CommandOutput> {
        use std::process::{Command, Stdio};
        
        let mut cmd = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command]);
            cmd
        } else {
            let mut cmd = Command::new("bash");
            cmd.args(["-c", command]);
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