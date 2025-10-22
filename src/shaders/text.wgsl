// Text rendering shaders for TimeLoop Terminal

// Vertex shader input
struct VertexInput {
    @location(0) position: vec2<f32>,
}

// Instance data
struct GlyphInstance {
    @location(1) pos: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) uv_rect: vec4<f32>,  // u0, v0, u1, v1
    @location(4) fg_color: u32,
    @location(5) flags: vec2<u16>,
    @location(6) time_created: f32,
}

// Uniform buffer
struct Uniforms {
    projection: mat4x4<f32>,
    time: f32,
    dpi_scale: f32,
    _padding: vec2<f32>,
}

// Vertex shader output
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fg_color: vec4<f32>,
    @location(2) time_created: f32,
}

// Bind group
@group(0) @binding(0) var atlas_texture: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

// Unpack RGBA8 color
fn unpack_rgba8(packed: u32) -> vec4<f32> {
    let r = f32((packed >> 24) & 0xFF) / 255.0;
    let g = f32((packed >> 16) & 0xFF) / 255.0;
    let b = f32((packed >> 8) & 0xFF) / 255.0;
    let a = f32(packed & 0xFF) / 255.0;
    return vec4<f32>(r, g, b, a);
}

// Vertex shader
@vertex
fn vs_main(vertex: VertexInput, instance: GlyphInstance) -> VertexOutput {
    // Transform vertex position to screen space
    let world_pos = instance.pos + vertex.position * instance.size;
    let clip_pos = uniforms.projection * vec4<f32>(world_pos, 0.0, 1.0);
    
    // Calculate UV coordinates
    let uv = mix(
        instance.uv_rect.xy,  // u0, v0
        instance.uv_rect.zw,  // u1, v1
        (vertex.position + 0.5)  // Convert from [-0.5, 0.5] to [0, 1]
    );
    
    // Unpack foreground color
    let fg_color = unpack_rgba8(instance.fg_color);
    
    return VertexOutput(
        clip_pos,
        uv,
        fg_color,
        instance.time_created
    );
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample the atlas texture
    let atlas_sample = textureSample(atlas_texture, atlas_sampler, in.uv);
    
    // For now, use simple alpha blending
    // In a real implementation, this would handle SDF-based anti-aliasing
    let alpha = atlas_sample.r;
    
    // Apply foreground color
    let color = in.fg_color * alpha;
    
    // Apply timeline effects based on time
    let time_diff = uniforms.time - in.time_created;
    let highlight_intensity = smoothstep(0.0, 1.0, 1.0 - time_diff * 0.1);
    
    // Add subtle highlight for recent text
    let highlight = vec4<f32>(0.2, 0.4, 1.0, 0.3) * highlight_intensity;
    let final_color = color + highlight;
    
    return final_color;
}