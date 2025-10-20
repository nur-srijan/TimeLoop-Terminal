use timeloop_terminal::gpu_renderer::*;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();
    
    // Create window
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new()
        .with_title("TimeLoop Terminal - GPU Text Demo")
        .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
        .build(&event_loop)?;
    
    // Create GPU renderer
    let mut renderer = GpuRenderer::new(&window).await?;
    
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