use timeloop_terminal::GpuRenderer;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::Window,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();
    
    // Create window
    let event_loop = EventLoop::new()?;
    let window_attributes = Window::default_attributes()
        .with_title("TimeLoop Terminal - GPU Text Demo")
        .with_inner_size(winit::dpi::LogicalSize::new(800, 600));

    let window = Arc::new(event_loop.create_window(window_attributes)?);
    
    // Create GPU renderer
    // GpuRenderer::new now likely takes `Arc<Window>` or `Window`, based on wgpu adapter changes.
    // The previous error was: `expected Window, found &Window`
    // So we should pass the window, but we also need it for the event loop.
    // However, `create_window` from `EventLoop` in newer winit might return `Window` not `Arc<Window>` or similar?
    // Wait, winit 0.30 changed API significantly.
    // `create_window` is on `ActiveEventLoop` which is passed to `run_app` or similar.
    // But here `EventLoop::new()` returns `EventLoop`.
    // The error message: `use of deprecated method winit::event_loop::EventLoop::<T>::create_window: use ActiveEventLoop::create_window instead`
    // And `GpuRenderer::new` expects `Window`.

    // Let's assume for now we can just fix the call site error `&window` -> `window`.
    // But since we use `window` later in the closure, and `Window` is not Copy/Clone (usually),
    // we might need to wrap it in Arc or move it.
    // But `GpuRenderer` likely needs ownership or a reference that lasts long enough.
    // The error said: `expected Window, found &Window`. This implies it wants ownership.
    // If it takes ownership, we can't use `window` in the event loop closure easily unless we share it via Arc.
    // Let's check `src/gpu_renderer.rs` to see what it expects.
    let mut renderer = GpuRenderer::new(window.clone()).await?;
    
    // Demo text
    let demo_text = "Hello, TimeLoop Terminal!\nThis is GPU-rendered text.\nIt supports multiple lines and should be smooth.";
    
    let mut time = 0.0;
    
    // Run event loop
    event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent {
                ref event,
                window_id,
            } if window_id == window.id() => match event {
                WindowEvent::CloseRequested => {
                    elwt.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    renderer.resize(physical_size.width, physical_size.height);
                }
                WindowEvent::RedrawRequested => {
                    // Render the demo text
                    if let Err(e) = renderer.render(demo_text, time) {
                        eprintln!("Render error: {}", e);
                    }
                    time += 0.016; // ~60 FPS
                }
                _ => {}
            },
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;
    
    Ok(())
}