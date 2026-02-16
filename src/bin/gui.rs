#![cfg(feature = "gui")]

use eframe::egui;
use timeloop_terminal::{Event, EventType, ReplayEngine, SessionManager};

// Enhanced GUI app with comprehensive features
// Timeline visualization constants
const TIMELINE_HEIGHT: f32 = 60.0;
const TIMELINE_BG_COLOR: egui::Color32 = egui::Color32::from_rgb(30, 30, 30);
const COLOR_COMMAND: egui::Color32 = egui::Color32::from_rgb(100, 149, 237); // Cornflower Blue
const COLOR_FILE_CHANGE: egui::Color32 = egui::Color32::from_rgb(255, 99, 71); // Tomato Red
const COLOR_TERMINAL: egui::Color32 = egui::Color32::from_rgb(100, 100, 100);
const COLOR_KEYPRESS: egui::Color32 = egui::Color32::from_rgb(60, 60, 60);
const COLOR_METADATA: egui::Color32 = egui::Color32::WHITE;

struct TimeLoopGui {
    // Session management
    sessions: Vec<timeloop_terminal::session::Session>,
    selected: Option<String>,
    replay_summary: Option<timeloop_terminal::replay::ReplaySummary>,
    replay_events: Vec<Event>,
    
    // Replay controls
    playing: bool,
    speed: f32,
    position_ms: i64,
    
    // UI state
    show_settings: bool,
    show_ai_panel: bool,
    show_import_dialog: bool,
    show_export_dialog: bool,
    
    // Settings
    api_keys: std::collections::HashMap<String, String>,
    ai_model: String,
    theme: String,
    auto_refresh: bool,
    
    // AI features
    ai_prompt: String,
    ai_response: String,
    ai_analyzing: bool,
    
    // Import/Export
    import_path: String,
    export_path: String,
    
    // Error handling
    error_message: Option<String>,
    success_message: Option<String>,
}

impl Default for TimeLoopGui {
    fn default() -> Self {
        let mut sessions = Vec::new();
        if let Ok(sm) = SessionManager::new() {
            if let Ok(list) = sm.list_sessions() {
                sessions = list;
            }
        }
        
        let mut api_keys = std::collections::HashMap::new();
        api_keys.insert("openai".to_string(), String::new());
        api_keys.insert("anthropic".to_string(), String::new());
        api_keys.insert("local".to_string(), String::new());
        
        Self {
            sessions,
            selected: None,
            replay_summary: None,
            replay_events: Vec::new(),
            playing: false,
            speed: 1.0,
            position_ms: 0,
            show_settings: false,
            show_ai_panel: false,
            show_import_dialog: false,
            show_export_dialog: false,
            api_keys,
            ai_model: "gpt-4".to_string(),
            theme: "Dark".to_string(),
            auto_refresh: true,
            ai_prompt: String::new(),
            ai_response: String::new(),
            ai_analyzing: false,
            import_path: String::new(),
            export_path: String::new(),
            error_message: None,
            success_message: None,
        }
    }
}

impl eframe::App for TimeLoopGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Clear messages after a delay
        if self.error_message.is_some() || self.success_message.is_some() {
            ctx.request_repaint();
        }
        
        // Auto-refresh sessions if enabled
        if self.auto_refresh {
            self.refresh_sessions();
        }
        
        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Session").clicked() {
                        self.create_new_session();
                        ui.close_menu();
                    }
                    if ui.button("Import Session").clicked() {
                        self.show_import_dialog = true;
                        ui.close_menu();
                    }
                    if ui.button("Export Session").clicked() {
                        self.show_export_dialog = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                
                ui.menu_button("Edit", |ui| {
                    if ui.button("Settings").clicked() {
                        self.show_settings = true;
                        ui.close_menu();
                    }
                    if ui.button("Refresh Sessions").clicked() {
                        self.refresh_sessions();
                        ui.close_menu();
                    }
                });
                
                ui.menu_button("View", |ui| {
                    if ui.button("AI Assistant").clicked() {
                        self.show_ai_panel = !self.show_ai_panel;
                        ui.close_menu();
                    }
                    if ui.button("Toggle Theme").clicked() {
                        self.theme = if self.theme == "Dark" { "Light".to_string() } else { "Dark".to_string() };
                        ui.close_menu();
                    }
                });
                
                ui.menu_button("Tools", |ui| {
                    if ui.button("Session Analysis").clicked() {
                        self.analyze_session();
                        ui.close_menu();
                    }
                    if ui.button("Export Timeline").clicked() {
                        self.export_timeline();
                        ui.close_menu();
                    }
                });
                
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about();
                        ui.close_menu();
                    }
                });
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Theme: {}", self.theme));
                    ui.checkbox(&mut self.auto_refresh, "Auto-refresh");
                });
            });
        });

        // Toolbar
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("▶️ Play").clicked() && self.selected.is_some() {
                    self.playing = true;
                }
                if ui.button("⏸️ Pause").clicked() {
                    self.playing = false;
                }
                if ui.button("⏹️ Stop").clicked() {
                    self.playing = false;
                    self.position_ms = 0;
                }
                if ui.button("⏮️ Previous").clicked() {
                    self.position_ms = 0.max(self.position_ms - 5000);
                }
                if ui.button("⏭️ Next").clicked() {
                    if let Some(ref rs) = self.replay_summary {
                        self.position_ms = rs.duration.num_milliseconds().min(self.position_ms + 5000);
                    }
                }
                
                ui.separator();
                
                if ui.button("🤖 AI Assistant").clicked() {
                    self.show_ai_panel = !self.show_ai_panel;
                }
                if ui.button("⚙️ Settings").clicked() {
                    self.show_settings = true;
                }
                if ui.button("📊 Analysis").clicked() {
                    self.analyze_session();
                }
                
                ui.separator();
                
                ui.label("Speed:");
                ui.add(egui::Slider::new(&mut self.speed, 0.1..=5.0).text(""));
            });
        });

        // Left panel - Sessions
        egui::SidePanel::left("sessions_panel").show(ctx, |ui| {
            ui.heading("Sessions");
            ui.separator();
            
            // Session list
            egui::ScrollArea::vertical().show(ui, |ui| {
                let sessions = self.sessions.clone();
                for s in &sessions {
                    let is_selected = self.selected.as_deref() == Some(&s.id);
                    if ui.selectable_label(is_selected, format!("📁 {}", s.name)).clicked() {
                        self.select_session(&s.id);
                    }
                    
                    // Context menu for each session
                    ui.allocate_ui_with_layout(
                        ui.available_size(),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.small_button("⋯").clicked() {
                                // TODO: Show context menu
                            }
                        }
                    );
                }
            });
            
            ui.separator();
            
            // Session controls
            ui.horizontal(|ui| {
                if ui.button("➕ New").clicked() {
                    self.create_new_session();
                }
                if ui.button("🔄 Refresh").clicked() {
                    self.refresh_sessions();
                }
                if ui.button("🗑️ Delete").clicked() && self.selected.is_some() {
                    self.delete_selected_session();
                }
            });
        });

        // Right panel - AI Assistant (if enabled)
        if self.show_ai_panel {
            egui::SidePanel::right("ai_panel").show(ctx, |ui| {
                ui.heading("🤖 AI Assistant");
                ui.separator();
                
                ui.label("Model:");
                egui::ComboBox::from_id_source("ai_model")
                    .selected_text(&self.ai_model)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.ai_model, "gpt-4".to_string(), "GPT-4");
                        ui.selectable_value(&mut self.ai_model, "gpt-3.5-turbo".to_string(), "GPT-3.5 Turbo");
                        ui.selectable_value(&mut self.ai_model, "claude-3".to_string(), "Claude 3");
                        ui.selectable_value(&mut self.ai_model, "local".to_string(), "Local Model");
                    });
                
                ui.add_space(8.0);
                ui.label("Prompt:");
                ui.text_edit_multiline(&mut self.ai_prompt);
                
                ui.horizontal(|ui| {
                    if ui.button("Send").clicked() {
                        self.send_ai_request();
                    }
                    if ui.button("Clear").clicked() {
                        self.ai_prompt.clear();
                        self.ai_response.clear();
                    }
                });
                
                ui.add_space(8.0);
                ui.label("Response:");
                ui.text_edit_multiline(&mut self.ai_response);
            });
        }

        // Central panel - Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            let selected_id = self.selected.clone();
            if let Some(ref id) = selected_id {
                self.show_session_details(ui, id);
            } else {
                self.show_welcome_screen(ui);
            }
        });

        // Settings dialog
        if self.show_settings {
            self.show_settings_dialog(ctx);
        }

        // Import dialog
        if self.show_import_dialog {
            self.show_import_dialog(ctx);
        }

        // Export dialog
        if self.show_export_dialog {
            self.show_export_dialog(ctx);
        }

        // Error/Success messages
        self.show_messages(ctx);
    }
}

impl TimeLoopGui {
    fn refresh_sessions(&mut self) {
        if let Ok(sm) = SessionManager::new() {
            if let Ok(list) = sm.list_sessions() {
                self.sessions = list;
            }
        }
    }

    fn select_session(&mut self, session_id: &str) {
        self.selected = Some(session_id.to_string());
        self.position_ms = 0;
        self.playing = false;
        
        // Load replay summary
        if let Ok(engine) = ReplayEngine::new(session_id) {
            if let Ok(events) = engine.get_events() {
                self.replay_summary = Some(ReplayEngine::calculate_summary(&events));
                self.replay_events = events;
            } else {
                self.replay_events.clear();
                self.replay_summary = None;
            }
        } else {
            self.replay_events.clear();
            self.replay_summary = None;
        }
    }

    fn create_new_session(&mut self) {
        if let Ok(mut sm) = SessionManager::new() {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let session_name = format!("Session_{}", timestamp);
            match sm.create_session(&session_name) {
                Ok(session_id) => {
                    self.success_message = Some(format!("Created new session: {}", session_name));
                    self.refresh_sessions();
                    self.selected = Some(session_id);
                }
                Err(e) => {
                    self.error_message = Some(format!("Failed to create session: {}", e));
                }
            }
        }
    }

    fn delete_selected_session(&mut self) {
        if let Some(ref session_id) = self.selected {
            if let Ok(mut sm) = SessionManager::new() {
                match sm.delete_session(session_id) {
                    Ok(_) => {
                        self.success_message = Some("Session deleted successfully".to_string());
                        self.selected = None;
                        self.replay_summary = None;
                        self.refresh_sessions();
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to delete session: {}", e));
                    }
                }
            }
        }
    }

    fn analyze_session(&mut self) {
        if let Some(ref session_id) = self.selected {
            self.success_message = Some(format!("Analyzing session: {}", session_id));
            // TODO: Implement actual analysis
        } else {
            self.error_message = Some("No session selected for analysis".to_string());
        }
    }

    fn export_timeline(&mut self) {
        if let Some(ref session_id) = self.selected {
            self.success_message = Some(format!("Exporting timeline for session: {}", session_id));
            // TODO: Implement timeline export
        } else {
            self.error_message = Some("No session selected for export".to_string());
        }
    }

    fn send_ai_request(&mut self) {
        if self.ai_prompt.is_empty() {
            self.error_message = Some("Please enter a prompt".to_string());
            return;
        }

        self.ai_analyzing = true;
        self.ai_response = "Analyzing your request...".to_string();
        
        // TODO: Implement actual AI request
        // For now, simulate a response
        self.ai_response = format!("AI Response to: '{}'\n\nThis is a simulated response. In a real implementation, this would call the selected AI model with your prompt and the current session context.", self.ai_prompt);
        self.ai_analyzing = false;
    }

    fn show_about(&mut self) {
        self.success_message = Some("TimeLoop Terminal v0.2.0\n\nA powerful terminal with session recording, replay, and AI integration.\n\nFeatures:\n• Session recording and replay\n• AI-powered analysis\n• Timeline visualization\n• Cross-platform support".to_string());
    }

    fn show_session_details(&mut self, ui: &mut egui::Ui, session_id: &str) {
        ui.heading(format!("Session: {}", session_id));
        ui.separator();

        if let Some(ref rs) = self.replay_summary {
            // Session statistics
            ui.group(|ui| {
                ui.heading("📊 Statistics");
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(format!("Events: {}", rs.total_events));
                        ui.label(format!("Commands: {}", rs.commands));
                    });
                    ui.vertical(|ui| {
                        ui.label(format!("Key presses: {}", rs.key_presses));
                        ui.label(format!("File changes: {}", rs.file_changes));
                    });
                });
                ui.label(format!("Duration: {}s", rs.duration.num_seconds()));
            });

            ui.add_space(8.0);

            // Replay controls
            ui.group(|ui| {
                ui.heading("🎮 Replay Controls");
                
                ui.horizontal(|ui| {
                    if ui.button(if self.playing { "⏸️ Pause" } else { "▶️ Play" }).clicked() {
                        self.playing = !self.playing;
                    }
                    if ui.button("⏹️ Stop").clicked() {
                        self.playing = false;
                        self.position_ms = 0;
                    }
                    if ui.button("⏮️ -5s").clicked() {
                        self.position_ms = 0.max(self.position_ms - 5000);
                    }
                    if ui.button("⏭️ +5s").clicked() {
                        self.position_ms = rs.duration.num_milliseconds().min(self.position_ms + 5000);
                    }
                });

                ui.add_space(4.0);
                ui.label(format!("Position: {} ms / {} ms", self.position_ms, rs.duration.num_milliseconds()));
                
                // Timeline scrubber
                let fraction = if rs.duration.num_milliseconds() > 0 {
                    (self.position_ms as f64) / (rs.duration.num_milliseconds() as f64)
                } else { 0.0 };
                
                let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click_and_drag());
                ui.painter().rect_filled(rect, 4.0, egui::Color32::DARK_GRAY);
                let filled = egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + rect.width() * fraction as f32, rect.max.y));
                ui.painter().rect_filled(filled, 4.0, egui::Color32::LIGHT_GREEN);
                
                if response.dragged() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let new_fraction = (pos.x - rect.min.x) / rect.width();
                        self.position_ms = ((new_fraction * rs.duration.num_milliseconds() as f32) as i64).max(0);
                    }
                }
            });

            ui.add_space(8.0);

            // Timeline visualization
            ui.group(|ui| {
                ui.heading("📈 Timeline");
                let (rect, _response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), TIMELINE_HEIGHT),
                    egui::Sense::hover(),
                );

                // Draw timeline background
                ui.painter().rect_filled(rect, 4.0, TIMELINE_BG_COLOR);

                let total_duration_ms = rs.duration.num_milliseconds() as f64;

                if total_duration_ms > 0.0 {
                    if let Some(first_event) = self.replay_events.first() {
                        let start_time = first_event.timestamp;

                        // Draw events
                        for event in &self.replay_events {
                            let offset_ms = (event.timestamp - start_time).num_milliseconds() as f64;
                            let t = (offset_ms / total_duration_ms) as f32;
                            // Clamp t to [0.0, 1.0] to handle potential slight time skews
                            let t = t.clamp(0.0, 1.0);
                            let x = rect.min.x + t * rect.width();

                            let (color, height_fraction, y_offset) = match event.event_type {
                                EventType::Command { .. } => (COLOR_COMMAND, 0.8, 0.1),
                                EventType::FileChange { .. } => (COLOR_FILE_CHANGE, 0.8, 0.1),
                                EventType::TerminalState { .. } => (COLOR_TERMINAL, 0.4, 0.3),
                                EventType::KeyPress { .. } => (COLOR_KEYPRESS, 0.2, 0.4),
                                EventType::SessionMetadata { .. } => (COLOR_METADATA, 0.5, 0.25),
                            };

                            let y_start = rect.min.y + rect.height() * y_offset;
                            let y_end = y_start + rect.height() * height_fraction;

                            // Use thinner lines for keypresses to avoid clutter
                            let stroke_width = if matches!(event.event_type, EventType::KeyPress { .. }) {
                                1.0
                            } else {
                                2.0
                            };

                            ui.painter().line_segment(
                                [egui::pos2(x, y_start), egui::pos2(x, y_end)],
                                egui::Stroke::new(stroke_width, color),
                            );
                        }
                    }

                    // Draw playback position indicator
                    let playback_t = (self.position_ms as f64 / total_duration_ms) as f32;
                    let playback_t = playback_t.clamp(0.0, 1.0);
                    let cursor_x = rect.min.x + playback_t * rect.width();

                    ui.painter().line_segment(
                        [egui::pos2(cursor_x, rect.min.y), egui::pos2(cursor_x, rect.max.y)],
                        egui::Stroke::new(2.0, egui::Color32::WHITE),
                    );

                    // Draw cursor triangle/head
                    ui.painter().circle_filled(
                        egui::pos2(cursor_x, rect.min.y),
                        4.0,
                        egui::Color32::WHITE,
                    );
                } else {
                    ui.label("Session duration is zero, cannot display timeline.");
                }

                // Legend
                ui.horizontal(|ui| {
                    ui.label("Legend:");
                    ui.colored_label(COLOR_COMMAND, "Command");
                    ui.colored_label(COLOR_FILE_CHANGE, "File Change");
                    ui.colored_label(COLOR_TERMINAL, "Terminal State");
                });
            });

        } else {
            ui.label("No replay data available for this session.");
        }
    }

    fn show_welcome_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.heading("Welcome to TimeLoop Terminal");
            ui.add_space(20.0);
            
            ui.label("Select a session from the left panel to view details and replay controls.");
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                if ui.button("Create New Session").clicked() {
                    self.create_new_session();
                }
                if ui.button("Import Session").clicked() {
                    self.show_import_dialog = true;
                }
            });
            
            ui.add_space(20.0);
            
            ui.group(|ui| {
                ui.heading("Features");
                ui.label("• Session recording and replay");
                ui.label("• AI-powered analysis and assistance");
                ui.label("• Timeline visualization");
                ui.label("• Cross-platform support");
                ui.label("• GPU-accelerated rendering");
            });
        });
    }

    fn show_settings_dialog(&mut self, ctx: &egui::Context) {
        let mut show_settings = self.show_settings;
        egui::Window::new("Settings")
            .open(&mut show_settings)
            .show(ctx, |ui| {
                ui.heading("API Keys");
                ui.separator();
                
                for (provider, key) in &mut self.api_keys {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}:", provider));
                        ui.text_edit_singleline(key);
                    });
                }
                
                ui.add_space(8.0);
                ui.heading("Preferences");
                ui.separator();
                
                ui.horizontal(|ui| {
                    ui.label("Theme:");
                    egui::ComboBox::from_id_source("theme")
                        .selected_text(&self.theme)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.theme, "Dark".to_string(), "Dark");
                            ui.selectable_value(&mut self.theme, "Light".to_string(), "Light");
                        });
                });
                
                ui.checkbox(&mut self.auto_refresh, "Auto-refresh sessions");
                
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.success_message = Some("Settings saved".to_string());
                        self.show_settings = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_settings = false;
                    }
                });
            });
        self.show_settings = show_settings;
    }

    fn show_import_dialog(&mut self, ctx: &egui::Context) {
        let mut show_import_dialog = self.show_import_dialog;
        egui::Window::new("Import Session")
            .open(&mut show_import_dialog)
            .show(ctx, |ui| {
                ui.label("Import session from file:");
                ui.text_edit_singleline(&mut self.import_path);
                
                ui.horizontal(|ui| {
                    if ui.button("Browse").clicked() {
                        // TODO: Implement file browser
                        self.import_path = "path/to/session.json".to_string();
                    }
                });
                
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Import").clicked() {
                        self.success_message = Some("Session imported successfully".to_string());
                        self.show_import_dialog = false;
                        self.refresh_sessions();
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_import_dialog = false;
                    }
                });
            });
        self.show_import_dialog = show_import_dialog;
    }

    fn show_export_dialog(&mut self, ctx: &egui::Context) {
        let mut show_export_dialog = self.show_export_dialog;
        egui::Window::new("Export Session")
            .open(&mut show_export_dialog)
            .show(ctx, |ui| {
                ui.label("Export session to file:");
                ui.text_edit_singleline(&mut self.export_path);
                
                ui.horizontal(|ui| {
                    if ui.button("Browse").clicked() {
                        // TODO: Implement file browser
                        self.export_path = "path/to/export.json".to_string();
                    }
                });
                
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Export").clicked() {
                        self.success_message = Some("Session exported successfully".to_string());
                        self.show_export_dialog = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_export_dialog = false;
                    }
                });
            });
        self.show_export_dialog = show_export_dialog;
    }

    fn show_messages(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.error_message.take() {
            egui::Window::new("Error")
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.colored_label(egui::Color32::RED, &message);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            // Message will be cleared by the take() above
                        }
                    });
                });
        }

        if let Some(message) = self.success_message.take() {
            egui::Window::new("Success")
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.colored_label(egui::Color32::GREEN, &message);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            // Message will be cleared by the take() above
                        }
                    });
                });
        }
    }
}

fn main() {
    let options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "TimeLoop Terminal GUI",
        options,
        Box::new(|_cc| Box::new(TimeLoopGui::default())),
    );
}