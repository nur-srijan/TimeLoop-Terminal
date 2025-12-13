#![cfg(feature = "gui")]

use eframe::egui;
use timeloop_terminal::{ReplayEngine, SessionManager};

// Minimal GUI app that lists sessions and shows summary + simple replay controls
struct TimeLoopGui {
    sessions: Vec<timeloop_terminal::session::Session>,
    selected: Option<String>,
    replay_summary: Option<timeloop_terminal::replay::ReplaySummary>,
    playing: bool,
    speed: f32,
    position_ms: i64,
}

impl Default for TimeLoopGui {
    fn default() -> Self {
        let mut sessions = Vec::new();
        if let Ok(sm) = SessionManager::new() {
            if let Ok(list) = sm.list_sessions() {
                sessions = list;
            }
        }
        Self {
            sessions,
            selected: None,
            replay_summary: None,
            playing: false,
            speed: 1.0,
            position_ms: 0,
        }
    }
}

impl eframe::App for TimeLoopGui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("TimeLoop Terminal (GUI)");
        });

        egui::SidePanel::left("left_panel").show(ctx, |ui| {
            ui.label("Sessions:");
            ui.separator();
            for s in &self.sessions {
                if ui
                    .selectable_label(
                        self.selected.as_deref() == Some(&s.id),
                        format!("{} - {}", s.id, s.name),
                    )
                    .clicked()
                {
                    self.selected = Some(s.id.clone());
                    // load summary
                    if let Ok(sm) = SessionManager::new() {
                        if let Ok(summary) = sm.get_session_summary(&s.id) {
                            // store minimal Summary (not used directly by replay)
                            let _ = summary;
                        }
                    }
                    // try to load replay summary
                    if let Ok(engine) = ReplayEngine::new(&s.id) {
                        if let Ok(rs) = engine.get_session_summary() {
                            self.replay_summary = Some(rs);
                            // reset playback position
                            self.position_ms = 0;
                            self.playing = false;
                        }
                    }
                }
            }
            ui.add_space(8.0);
            if ui
                .button("Refresh")
                .on_hover_text("Reload the list of sessions")
                .clicked()
            {
                if let Ok(sm) = SessionManager::new() {
                    if let Ok(list) = sm.list_sessions() {
                        self.sessions = list;
                    }
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Session Details");
            ui.separator();
            if let Some(ref id) = self.selected {
                ui.label(format!("Selected: {}", id));
                if let Some(ref rs) = self.replay_summary {
                    ui.label(format!("Events: {}", rs.total_events));
                    ui.label(format!("Commands: {}", rs.commands));
                    ui.label(format!("Key presses: {}", rs.key_presses));
                    ui.label(format!("File changes: {}", rs.file_changes));
                    ui.label(format!("Duration: {}s", rs.duration.num_seconds()));

                    ui.horizontal(|ui| {
                        if ui
                            .button(if self.playing { "Pause" } else { "Play" })
                            .on_hover_text("Start or pause session playback")
                            .clicked()
                        {
                            self.playing = !self.playing;
                        }
                        if ui
                            .button("Step +1s")
                            .on_hover_text("Advance playback by 1 second")
                            .clicked()
                        {
                            self.position_ms += 1000;
                        }
                        ui.add(egui::Slider::new(&mut self.speed, 0.25..=4.0).text("Speed"))
                            .on_hover_text("Adjust playback speed (0.25x to 4.0x)");
                    });

                    ui.add_space(8.0);
                    ui.label(format!("Position: {} ms", self.position_ms));

                    // Simple timeline visualization
                    let fraction = if rs.duration.num_milliseconds() > 0 {
                        (self.position_ms as f64) / (rs.duration.num_milliseconds() as f64)
                    } else {
                        0.0
                    };
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 30.0),
                        egui::Sense::hover(),
                    );
                    response.on_hover_text(format!("Playback progress: {:.0}%", fraction * 100.0));

                    ui.painter()
                        .rect_filled(rect, 4.0, egui::Color32::DARK_GRAY);
                    let filled = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.min.x + rect.width() * fraction as f32, rect.max.y),
                    );
                    ui.painter()
                        .rect_filled(filled, 4.0, egui::Color32::LIGHT_GREEN);

                    // Playback advancement
                    if self.playing {
                        // advance position based on frame time and speed
                        // `ctx.input(|i| i.unstable_dt)` returns f32 (not Option), so use it and fallback to 1.0 if zero
                        let delta = ctx.input(|i| i.unstable_dt);
                        let delta = if delta == 0.0 { 1.0 } else { delta };
                        self.position_ms += ((delta * 1000.0) as f64 * (self.speed as f64)) as i64;
                        if self.position_ms > rs.duration.num_milliseconds() {
                            self.position_ms = rs.duration.num_milliseconds();
                            self.playing = false;
                        }
                        ctx.request_repaint();
                    }
                } else {
                    ui.label("No replay summary available for this session.");
                }
            } else {
                ui.label("Select a session from the left sidebar to view details.");
            }
        });
    }
}

fn main() {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "TimeLoop Terminal GUI",
        options,
        Box::new(|_cc| Box::new(TimeLoopGui::default())),
    );
}
