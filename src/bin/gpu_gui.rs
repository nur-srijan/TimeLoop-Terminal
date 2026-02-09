#![cfg(feature = "gui")]

use eframe::egui;
use timeloop_terminal::{GpuRenderer, ReplayEngine, SessionManager};
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
};

// GPU-enabled GUI app that demonstrates text rendering
struct TimeLoopGpuGui {
    sessions: Vec<timeloop_terminal::session::Session>,
    selected: Option<String>,
    replay_summary: Option<timeloop_terminal::replay::ReplaySummary>,
    playing: bool,
    speed: f32,
    position_ms: i64,
    gpu_renderer: Option<GpuRenderer>,
    demo_text: String,
    time: f32,
}

impl Default for TimeLoopGpuGui {
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
            gpu_renderer: None,
            demo_text: "Hello, TimeLoop Terminal! This is GPU-rendered text.".to_string(),
            time: 0.0,
        }
    }
}

impl eframe::App for TimeLoopGpuGui {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Update time for animations
        self.time += ctx.input(|i| i.unstable_dt);

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("TimeLoop Terminal (GPU Rendering Demo)");
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
            if ui.button("Refresh").clicked() {
                if let Ok(sm) = SessionManager::new() {
                    if let Ok(list) = sm.list_sessions() {
                        self.sessions = list;
                    }
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("GPU Text Rendering Demo");
            ui.separator();

            // Demo text input
            ui.label("Demo Text:");
            ui.text_edit_multiline(&mut self.demo_text);

            ui.add_space(8.0);

            // GPU renderer status
            if let Some(_renderer) = &self.gpu_renderer {
                ui.label("✅ GPU Renderer: Active");
            } else {
                ui.label("❌ GPU Renderer: Not initialized");
                if ui.button("Initialize GPU Renderer").clicked() {
                    // This would initialize the GPU renderer in a real implementation
                    ui.label("GPU renderer initialization would happen here");
                }
            }

            ui.add_space(8.0);

            if let Some(ref id) = self.selected {
                ui.label(format!("Selected Session: {}", id));
                if let Some(ref rs) = self.replay_summary {
                    ui.label(format!("Events: {}", rs.total_events));
                    ui.label(format!("Commands: {}", rs.commands));
                    ui.label(format!("Key presses: {}", rs.key_presses));
                    ui.label(format!("File changes: {}", rs.file_changes));
                    ui.label(format!("Duration: {}s", rs.duration.num_seconds()));

                    ui.horizontal(|ui| {
                        if ui
                            .button(if self.playing { "Pause" } else { "Play" })
                            .clicked()
                        {
                            self.playing = !self.playing;
                        }
                        if ui.button("Step +1s").clicked() {
                            self.position_ms += 1000;
                        }
                        ui.add(egui::Slider::new(&mut self.speed, 0.25..=4.0).text("Speed"));
                    });

                    ui.add_space(8.0);
                    ui.label(format!("Position: {} ms", self.position_ms));

                    // Simple timeline visualization
                    let fraction = if rs.duration.num_milliseconds() > 0 {
                        (self.position_ms as f64) / (rs.duration.num_milliseconds() as f64)
                    } else {
                        0.0
                    };
                    let (rect, _response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 30.0),
                        egui::Sense::hover(),
                    );
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
                ui.label("No session selected.");
            }

            ui.add_space(16.0);
            ui.label("GPU Text Rendering Features:");
            ui.label("• HarfBuzz text shaping for complex scripts");
            ui.label("• Dynamic glyph atlas with LRU eviction");
            ui.label("• Instanced rendering for performance");
            ui.label("• Timeline-driven animations");
            ui.label("• SDF-based anti-aliasing (planned)");
            ui.label("• Multi-font support (planned)");
        });

        // Request repaint for animations
        ctx.request_repaint();
    }
}

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("TimeLoop Terminal - GPU Rendering Demo"),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "TimeLoop Terminal GPU Demo",
        options,
        Box::new(|_cc| Box::new(TimeLoopGpuGui::default())),
    );
}
