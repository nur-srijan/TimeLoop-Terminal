use crate::{Event, EventType, FileChangeType, Storage};
use crossterm::event::{self, Event as CEvent, KeyCode};
use crossterm::{
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
    ExecutableCommand,
};
use std::io::Write;
use std::time::Duration;
use tokio::time::{sleep, Instant};

pub struct ReplayEngine {
    storage: Storage,
    session_id: String,
}

impl ReplayEngine {
    pub fn new(session_id: &str) -> crate::Result<Self> {
        let storage = Storage::new()?;
        Ok(Self {
            storage,
            session_id: session_id.to_string(),
        })
    }

    pub async fn replay(&self, speed: f32) -> crate::Result<()> {
        let events = self.storage.get_events_for_session(&self.session_id)?;

        if events.is_empty() {
            println!("No events found for session: {}", self.session_id);
            return Ok(());
        }

        println!(
            "🎥 Replaying session: {} at {}x speed",
            self.session_id, speed
        );
        println!("Controls: space=pause/resume, +/-=speed, q=quit");
        println!("{}", "─".repeat(60));

        let _stdout = std::io::stdout();
        let mut last_timestamp = events[0].timestamp;
        let mut current_speed = if speed <= 0.0 { 1.0 } else { speed };
        let mut paused = false;

        for (i, event) in events.iter().enumerate() {
            // Calculate delay based on speed
            let delay = if i > 0 {
                let time_diff = event.timestamp - last_timestamp;
                let delay_ms = time_diff.num_milliseconds() as u64;
                (delay_ms as f32 / current_speed) as u64
            } else {
                0
            };

            if delay > 0 {
                let start = Instant::now();
                while start.elapsed().as_millis() < delay as u128 {
                    // Handle interactive input during delay
                    if event::poll(Duration::from_millis(50))? {
                        if let CEvent::Key(key) = event::read()? {
                            match key.code {
                                KeyCode::Char(' ') => {
                                    paused = !paused;
                                }
                                KeyCode::Char('+') => {
                                    current_speed *= 2.0;
                                }
                                KeyCode::Char('-') => {
                                    current_speed = (current_speed / 2.0).max(0.25);
                                }
                                KeyCode::Char('q') => {
                                    println!("\n⏹️  Quit replay");
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }
                    }
                    if paused {
                        sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    sleep(Duration::from_millis(10)).await;
                }
            }

            // Display the event
            self.display_event(event, i + 1, events.len())?;

            last_timestamp = event.timestamp;
        }

        println!("\n✅ Replay completed!");
        Ok(())
    }

    fn display_event(
        &self,
        event: &Event,
        event_num: usize,
        total_events: usize,
    ) -> crate::Result<()> {
        let mut stdout = std::io::stdout();

        // Clear the current line
        stdout.execute(Clear(ClearType::CurrentLine))?;

        // Show progress
        stdout.execute(SetForegroundColor(Color::Cyan))?;
        stdout.execute(Print(format!("[{}/{}] ", event_num, total_events)))?;
        stdout.execute(ResetColor)?;

        // Show timestamp
        stdout.execute(SetForegroundColor(Color::Yellow))?;
        stdout.execute(Print(format!("{} ", event.timestamp.format("%H:%M:%S"))))?;
        stdout.execute(ResetColor)?;

        match &event.event_type {
            EventType::KeyPress { key, .. } => {
                stdout.execute(SetForegroundColor(Color::Green))?;
                stdout.execute(Print("⌨️  "))?;
                stdout.execute(ResetColor)?;
                stdout.execute(Print(format!("Key: {}", key)))?;
            }
            EventType::Command {
                command,
                output,
                exit_code,
                working_directory,
                ..
            } => {
                stdout.execute(SetForegroundColor(Color::Blue))?;
                stdout.execute(Print("💻 "))?;
                stdout.execute(ResetColor)?;
                stdout.execute(Print(format!("Command: {}", command)))?;
                stdout.execute(Print("\n"))?;

                if !output.is_empty() {
                    stdout.execute(SetForegroundColor(Color::DarkGrey))?;
                    stdout.execute(Print("   Output: "))?;
                    stdout.execute(ResetColor)?;
                    stdout.execute(Print(output))?;
                }

                stdout.execute(SetForegroundColor(Color::Magenta))?;
                stdout.execute(Print(format!(
                    "   Exit: {}, Dir: {}",
                    exit_code, working_directory
                )))?;
                stdout.execute(ResetColor)?;
            }
            EventType::FileChange {
                path, change_type, ..
            } => {
                stdout.execute(SetForegroundColor(Color::Red))?;
                stdout.execute(Print("📁 "))?;
                stdout.execute(ResetColor)?;
                stdout.execute(Print(format!(
                    "File {}: {}",
                    match change_type {
                        FileChangeType::Created => "created",
                        FileChangeType::Modified => "modified",
                        FileChangeType::Deleted => "deleted",
                        FileChangeType::Renamed { .. } => "renamed",
                    },
                    path
                )))?;
            }
            EventType::TerminalState {
                cursor_position,
                screen_size,
                ..
            } => {
                stdout.execute(SetForegroundColor(Color::DarkGrey))?;
                stdout.execute(Print("🖥️  "))?;
                stdout.execute(ResetColor)?;
                stdout.execute(Print(format!(
                    "Terminal: {}x{}, cursor at {:?}",
                    screen_size.0, screen_size.1, cursor_position
                )))?;
            }
            EventType::SessionMetadata { name, .. } => {
                stdout.execute(SetForegroundColor(Color::White))?;
                stdout.execute(Print("📝 "))?;
                stdout.execute(ResetColor)?;
                stdout.execute(Print(format!("Session: {}", name)))?;
            }
        }

        stdout.execute(Print("\n"))?;
        stdout.flush()?;
        Ok(())
    }

    pub async fn replay_range(
        &self,
        start_time: chrono::DateTime<chrono::Utc>,
        end_time: chrono::DateTime<chrono::Utc>,
        speed: f32,
    ) -> crate::Result<()> {
        let events = self
            .storage
            .get_events_in_range(&self.session_id, start_time, end_time)?;

        if events.is_empty() {
            println!("No events found in the specified time range");
            return Ok(());
        }

        println!(
            "🎥 Replaying session range: {} to {} at {}x speed",
            start_time.format("%H:%M:%S"),
            end_time.format("%H:%M:%S"),
            speed
        );

        let mut last_timestamp = events[0].timestamp;

        for (i, event) in events.iter().enumerate() {
            let delay = if i > 0 {
                let time_diff = event.timestamp - last_timestamp;
                let delay_ms = time_diff.num_milliseconds() as u64;
                (delay_ms as f32 / speed) as u64
            } else {
                0
            };

            if delay > 0 {
                sleep(Duration::from_millis(delay)).await;
            }

            self.display_event(event, i + 1, events.len())?;
            last_timestamp = event.timestamp;
        }

        Ok(())
    }

    pub fn get_session_summary(&self) -> crate::Result<ReplaySummary> {
        let events = self.storage.get_events_for_session(&self.session_id)?;

        let mut commands = 0;
        let mut key_presses = 0;
        let mut file_changes = 0;
        let mut duration = chrono::Duration::zero();

        if let (Some(first), Some(last)) = (events.first(), events.last()) {
            duration = last.timestamp - first.timestamp;
        }

        for event in &events {
            match &event.event_type {
                EventType::Command { .. } => commands += 1,
                EventType::KeyPress { .. } => key_presses += 1,
                EventType::FileChange { .. } => file_changes += 1,
                _ => {}
            }
        }

        Ok(ReplaySummary {
            total_events: events.len(),
            commands,
            key_presses,
            file_changes,
            duration,
        })
    }
}

#[derive(Debug)]
pub struct ReplaySummary {
    pub total_events: usize,
    pub commands: usize,
    pub key_presses: usize,
    pub file_changes: usize,
    pub duration: chrono::Duration,
}
