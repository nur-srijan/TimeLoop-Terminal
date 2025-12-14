use std::collections::HashMap;
use std::sync::Arc;
use std::path::Path;
use wgpu::*;
use winit::window::Window;
use glam::Mat4;
use bytemuck::{Pod, Zeroable};
use crate::TimeLoopError;

/// Core GPU renderer for text rendering with wgpu
pub struct GpuRenderer {
    device: Arc<Device>,
    queue: Arc<Queue>,
    surface: Surface<'static>,
    surface_config: SurfaceConfiguration,
    render_pipeline: RenderPipeline,
    instance_buffer: Buffer,
    uniform_buffer: Buffer,
    bind_group: BindGroup,
    glyph_atlas: GlyphAtlas,
    text_shaper: TextShaper,
}

/// Glyph instance data for instanced rendering
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct GlyphInstance {
    pub pos: [f32; 2],           // x, y position in pixels
    pub size: [f32; 2],          // width, height
    pub uv_rect: [f32; 4],       // u0, v0, u1, v1 texture coordinates
    pub fg_color: u32,           // packed RGBA8 foreground color
    pub flags: u16,              // bold/italic/underline/emoji flags
    pub time_created: f32,       // timestamp for timeline effects
    pub _padding: u16,           // padding for alignment
}

unsafe impl Pod for GlyphInstance {}
unsafe impl Zeroable for GlyphInstance {}

/// Uniform buffer data
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Uniforms {
    pub projection: Mat4,
    pub time: f32,
    pub dpi_scale: f32,
    pub _padding: [f32; 2],
}

unsafe impl Pod for Uniforms {}
unsafe impl Zeroable for Uniforms {}

/// Glyph atlas manager for storing and managing glyph bitmaps
pub struct GlyphAtlas {
    texture: Texture,
    sampler: Sampler,
    width: u32,
    height: u32,
    slots: HashMap<GlyphKey, AtlasSlot>,
    packer: SkylinePacker,
    generation: u32,
    rasterizer: GlyphRasterizer,
}

/// Key for identifying glyphs in the atlas
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphKey {
    pub font_hash: u64,
    pub glyph_id: u32,
    pub size: u32,
    pub scale: f32,
}

impl Eq for GlyphKey {}

impl std::hash::Hash for GlyphKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.font_hash.hash(state);
        self.glyph_id.hash(state);
        self.size.hash(state);
        // Convert f32 to u32 for hashing
        self.scale.to_bits().hash(state);
    }
}

/// Atlas slot information
#[derive(Debug, Clone)]
pub struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub generation: u32,
}

/// Simple skyline packer for atlas management
pub struct SkylinePacker {
    skyline: Vec<u32>,
    width: u32,
    height: u32,
}

/// Text shaper using HarfBuzz
pub struct TextShaper {
    // HarfBuzz context will be added here
}

/// Simple glyph rasterizer (placeholder for FreeType integration)
pub struct GlyphRasterizer {
    // Placeholder for future FreeType integration
}

/// Rasterized glyph data
pub struct RasterizedGlyph {
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance: i32,
    pub pixels: Vec<u8>,
}

/// Text layout and shaping result
pub struct ShapedText {
    pub glyphs: Vec<GlyphPlacement>,
    pub width: f32,
    pub height: f32,
}

/// Individual glyph placement information
pub struct GlyphPlacement {
    pub glyph_id: u32,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    pub cluster: u32,
    pub font_key: GlyphKey,
}

impl GpuRenderer {
    /// Create a new GPU renderer
    pub async fn new(window: Window) -> Result<Self, TimeLoopError> {
        let size = window.inner_size();
        
        // Initialize wgpu
        let instance = Instance::new(InstanceDescriptor::default());
        let surface = instance.create_surface(window).map_err(|e| TimeLoopError::GpuError(e.to_string()))?;
        
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| TimeLoopError::GpuError("Failed to find suitable GPU adapter".to_string()))?;
        
        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    label: None,
                    required_features: Features::empty(),
                    required_limits: Limits::default(),
                },
                None,
            )
            .await
            .map_err(|e| TimeLoopError::GpuError(e.to_string()))?;
        
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        
        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        
        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);
        
        // Create glyph atlas
        let mut glyph_atlas = GlyphAtlas::new(&device, 4096, 4096)?;
        
        // Load default font (if available)
        if let Ok(font_path) = std::env::var("FONT_PATH") {
            let _ = glyph_atlas.load_font(Path::new(&font_path));
        }
        
        // Create text shaper
        let text_shaper = TextShaper::new()?;
        
        // Create render pipeline
        let render_pipeline = Self::create_render_pipeline(&device, surface_format)?;
        
        // Create buffers
        let instance_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("Instance Buffer"),
            size: std::mem::size_of::<GlyphInstance>() as u64 * 10000, // 10k instances
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("Uniform Buffer"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        // Create bind group
        let bind_group = Self::create_bind_group(&device, &glyph_atlas.texture, &glyph_atlas.sampler, &uniform_buffer)?;
        
        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
            render_pipeline,
            instance_buffer,
            uniform_buffer,
            bind_group,
            glyph_atlas,
            text_shaper,
        })
    }
    
    /// Create the render pipeline for text rendering
    fn create_render_pipeline(device: &Device, surface_format: TextureFormat) -> Result<RenderPipeline, TimeLoopError> {
        // Load shaders
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Text Shader"),
            source: ShaderSource::Wgsl(include_str!("shaders/text.wgsl").into()),
        });
        
        // Vertex buffer layouts
        let vertex_buffer_layouts = [
            // Unit quad vertices
            VertexBufferLayout {
                array_stride: std::mem::size_of::<[f32; 2]>() as u64,
                step_mode: VertexStepMode::Vertex,
                attributes: &[VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: VertexFormat::Float32x2,
                }],
            },
            // Instance data
            VertexBufferLayout {
                array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                step_mode: VertexStepMode::Instance,
                attributes: &[
                    VertexAttribute {
                        offset: std::mem::offset_of!(GlyphInstance, pos) as u64,
                        shader_location: 1,
                        format: VertexFormat::Float32x2,
                    },
                    VertexAttribute {
                        offset: std::mem::offset_of!(GlyphInstance, size) as u64,
                        shader_location: 2,
                        format: VertexFormat::Float32x2,
                    },
                    VertexAttribute {
                        offset: std::mem::offset_of!(GlyphInstance, uv_rect) as u64,
                        shader_location: 3,
                        format: VertexFormat::Float32x4,
                    },
                    VertexAttribute {
                        offset: std::mem::offset_of!(GlyphInstance, fg_color) as u64,
                        shader_location: 4,
                        format: VertexFormat::Uint32,
                    },
                    VertexAttribute {
                        offset: std::mem::offset_of!(GlyphInstance, flags) as u64,
                        shader_location: 5,
                        format: VertexFormat::Uint16x2,
                    },
                    VertexAttribute {
                        offset: std::mem::offset_of!(GlyphInstance, time_created) as u64,
                        shader_location: 6,
                        format: VertexFormat::Float32,
                    },
                ],
            },
        ];
        
        let render_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Text Render Pipeline Layout"),
            bind_group_layouts: &[&Self::create_bind_group_layout(device)?],
            push_constant_ranges: &[],
        });
        
        let render_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Text Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &vertex_buffer_layouts,
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(ColorTargetState {
                    format: surface_format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: PipelineCompilationOptions::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Back),
                polygon_mode: PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });
        
        Ok(render_pipeline)
    }
    
    /// Create bind group layout
    fn create_bind_group_layout(device: &Device) -> Result<BindGroupLayout, TimeLoopError> {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Text Bind Group Layout"),
            entries: &[
                // Atlas texture
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        multisampled: false,
                        view_dimension: TextureViewDimension::D2,
                        sample_type: TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // Atlas sampler
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                // Uniform buffer
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        
        Ok(layout)
    }
    
    /// Create bind group
    fn create_bind_group(
        device: &Device,
        atlas_texture: &Texture,
        atlas_sampler: &Sampler,
        uniform_buffer: &Buffer,
    ) -> Result<BindGroup, TimeLoopError> {
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Text Bind Group"),
            layout: &Self::create_bind_group_layout(device)?,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&atlas_texture.create_view(&TextureViewDescriptor::default())),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(atlas_sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });
        
        Ok(bind_group)
    }
    
    /// Render a frame with text
    pub fn render(&mut self, text: &str, time: f32) -> Result<(), TimeLoopError> {
        // Shape text
        let shaped_text = self.text_shaper.shape_text(text)?;
        
        // Ensure glyphs are in atlas
        for glyph in &shaped_text.glyphs {
            if !self.glyph_atlas.contains(&glyph.font_key) {
                self.glyph_atlas.add_glyph(&self.device, &self.queue, &glyph.font_key)?;
            }
        }
        
        // Build instance data
        let mut instances = Vec::new();
        for glyph in &shaped_text.glyphs {
            if let Some(slot) = self.glyph_atlas.get_slot(&glyph.font_key) {
                instances.push(GlyphInstance {
                    pos: [glyph.x, glyph.y],
                    size: [slot.width as f32, slot.height as f32],
                    uv_rect: [slot.u0, slot.v0, slot.u1, slot.v1],
                    fg_color: 0xFF_FF_FF_FF, // White text for now
                    flags: 0,
                    time_created: time,
                    _padding: 0,
                });
            }
        }
        
        // Update instance buffer
        if !instances.is_empty() {
            self.queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&instances),
            );
        }
        
        // Update uniform buffer
        let uniforms = Uniforms {
            projection: Mat4::orthographic_rh_gl(
                0.0,
                self.surface_config.width as f32,
                self.surface_config.height as f32,
                0.0,
                -1.0,
                1.0,
            ),
            time,
            dpi_scale: 1.0,
            _padding: [0.0; 2],
        };
        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );
        
        // Render
        let output = self.surface.get_current_texture().map_err(|e| TimeLoopError::GpuError(e.to_string()))?;
        let view = output.texture.create_view(&TextureViewDescriptor::default());
        
        // Unit quad vertices (hardcoded for now)
        let quad_vertices = [
            -0.5, -0.5,
             0.5, -0.5,
             0.5,  0.5,
            -0.5, -0.5,
             0.5,  0.5,
            -0.5,  0.5,
        ];
        let quad_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("Quad Buffer"),
            size: (quad_vertices.len() * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&quad_buffer, 0, bytemuck::cast_slice(&quad_vertices));

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Text Render Encoder"),
        });
        
        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Text Render Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            
            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.set_vertex_buffer(0, quad_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            
            if !instances.is_empty() {
                render_pass.draw(0..6, 0..instances.len() as u32);
            }
        }
        
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        
        Ok(())
    }
    
    /// Resize the surface
    pub fn resize(&mut self, width: u32, height: u32) {
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }
}

impl GlyphAtlas {
    /// Create a new glyph atlas
    pub fn new(device: &Device, width: u32, height: u32) -> Result<Self, TimeLoopError> {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Atlas Sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            ..Default::default()
        });
        
        let rasterizer = GlyphRasterizer::new()?;
        
        Ok(Self {
            texture,
            sampler,
            width,
            height,
            slots: HashMap::new(),
            packer: SkylinePacker::new(width, height),
            generation: 0,
            rasterizer,
        })
    }
    
    /// Check if a glyph exists in the atlas
    pub fn contains(&self, key: &GlyphKey) -> bool {
        self.slots.contains_key(key)
    }
    
    /// Get atlas slot for a glyph
    pub fn get_slot(&self, key: &GlyphKey) -> Option<&AtlasSlot> {
        self.slots.get(key)
    }
    
    /// Add a glyph to the atlas
    pub fn add_glyph(&mut self, _device: &Device, queue: &Queue, key: &GlyphKey) -> Result<(), TimeLoopError> {
        // Rasterize the glyph using FreeType
        let rasterized = self.rasterizer.rasterize_glyph(
            &format!("{:x}", key.font_hash),
            key.glyph_id,
            key.size,
            key.scale,
        )?;
        
        if let Some(rect) = self.packer.pack(rasterized.width, rasterized.height) {
            let slot = AtlasSlot {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                u0: rect.x as f32 / self.width as f32,
                v0: rect.y as f32 / self.height as f32,
                u1: (rect.x + rect.width) as f32 / self.width as f32,
                v1: (rect.y + rect.height) as f32 / self.height as f32,
                generation: self.generation,
            };
            
            // Upload to GPU
            queue.write_texture(
                ImageCopyTexture {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: Origin3d { x: rect.x, y: rect.y, z: 0 },
                    aspect: TextureAspect::All,
                },
                &rasterized.pixels,
                ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(rasterized.width * 4), // RGBA
                    rows_per_image: Some(rasterized.height),
                },
                Extent3d {
                    width: rasterized.width,
                    height: rasterized.height,
                    depth_or_array_layers: 1,
                },
            );
            
            self.slots.insert(key.clone(), slot);
            self.generation += 1;
        }
        
        Ok(())
    }
    
    /// Load a font into the rasterizer (placeholder)
    pub fn load_font(&mut self, _path: &std::path::Path) -> Result<u64, TimeLoopError> {
        // Placeholder implementation
        Ok(0)
    }
}

impl SkylinePacker {
    /// Create a new skyline packer
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            skyline: vec![0; width as usize],
            width,
            height,
        }
    }
    
    /// Pack a rectangle into the atlas
    pub fn pack(&mut self, width: u32, height: u32) -> Option<AtlasRect> {
        // Simple skyline packing algorithm
        for x in 0..=(self.width - width) {
            let mut max_height = 0;
            let mut can_fit = true;
            
            for i in 0..width as usize {
                let skyline_height = self.skyline[(x + i as u32) as usize];
                max_height = max_height.max(skyline_height);
                if skyline_height + height > self.height {
                    can_fit = false;
                    break;
                }
            }
            
            if can_fit {
                // Found a spot, update skyline
                for i in 0..width as usize {
                    self.skyline[(x + i as u32) as usize] = max_height + height;
                }
                
                return Some(AtlasRect {
                    x,
                    y: max_height,
                    width,
                    height,
                });
            }
        }
        
        None
    }
}

impl TextShaper {
    /// Create a new text shaper
    pub fn new() -> Result<Self, TimeLoopError> {
        // For now, create a simple shaper
        // In a real implementation, this would initialize HarfBuzz
        Ok(Self {})
    }
    
    /// Shape text into glyph placements
    pub fn shape_text(&self, text: &str) -> Result<ShapedText, TimeLoopError> {
        // For now, create simple glyph placements
        // In a real implementation, this would use HarfBuzz
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        let y = 0.0;
        
        for (i, ch) in text.chars().enumerate() {
            let glyph_key = GlyphKey {
                font_hash: 0, // Placeholder
                glyph_id: ch as u32,
                size: 16,
                scale: 1.0,
            };
            
            glyphs.push(GlyphPlacement {
                glyph_id: ch as u32,
                x,
                y,
                advance: 16.0,
                cluster: i as u32,
                font_key: glyph_key,
            });
            
            x += 16.0;
        }
        
        Ok(ShapedText {
            glyphs,
            width: x,
            height: 16.0,
        })
    }
}

impl GlyphRasterizer {
    /// Create a new glyph rasterizer
    pub fn new() -> Result<Self, TimeLoopError> {
        // Placeholder implementation
        Ok(Self {})
    }
    
    /// Load a font face (placeholder)
    pub fn load_font(&mut self, _path: &std::path::Path, _face_index: i32) -> Result<String, TimeLoopError> {
        // Placeholder implementation
        Ok("default_font".to_string())
    }
    
    /// Rasterize a glyph (placeholder)
    pub fn rasterize_glyph(&mut self, _font_key: &str, _glyph_id: u32, size: u32, _scale: f32) -> Result<RasterizedGlyph, TimeLoopError> {
        // Create a simple placeholder glyph (white square)
        let width = size;
        let height = size;
        let mut pixels = Vec::new();
        
        for _y in 0..height {
            for _x in 0..width {
                pixels.extend_from_slice(&[255, 255, 255, 255]); // White RGBA
            }
        }
        
        Ok(RasterizedGlyph {
            width,
            height,
            bearing_x: 0,
            bearing_y: size as i32,
            advance: size as i32,
            pixels,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AtlasRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

// Extend TimeLoopError to include GPU errors
impl From<wgpu::Error> for TimeLoopError {
    fn from(err: wgpu::Error) -> Self {
        TimeLoopError::GpuError(err.to_string())
    }
}

impl From<winit::error::OsError> for TimeLoopError {
    fn from(err: winit::error::OsError) -> Self {
        TimeLoopError::GpuError(err.to_string())
    }
}

impl From<winit::error::EventLoopError> for TimeLoopError {
    fn from(err: winit::error::EventLoopError) -> Self {
        TimeLoopError::GpuError(err.to_string())
    }
}