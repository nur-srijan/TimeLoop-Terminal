# GPU text rendering for **TimeLoop-Terminal** — architecture outline (using **wgpu**)

---

# 1) High-level flow (one-paragraph cheat sheet)

1. Shape strings into glyph runs (HarfBuzz) → get glyph IDs, positions, clusters.
2. Ensure glyph bitmaps (or SDF) exist in the GPU-backed glyph atlas; if not, rasterize with FreeType and upload a sub-region.
3. For each visible glyph, append an instance/quad to a CPU-side vertex/instance buffer (x,y,uv rect, color, extra flags).
4. Upload (or map) instance buffer to GPU, issue a single draw call or a few large draws for the render pass.
5. Fragment shader samples atlas, applies AA (or SDF smoothing), composes with terminal cell background + overlays.
6. Sync with TimeLoop playback: glyph appearance = timeline event → render highlighting/animations driven by the frame/time uniform.

---

# 2) Major components & responsibilities

**A. Text shaping & layout (CPU)**

* HarfBuzz for shaping (ligatures, complex scripts, RTL).
* Maintain line wrapping, cursor location, selection spans.
* Emit glyph runs: `Glyph { id, x_offset, y_offset, advance, cluster }`.

**B. Glyph rasterizer / atlas manager (CPU + GPU)**

* Uses FreeType (or Rust wrappers) to rasterize glyphs into either:

  * *Bitmap masks* (alpha), OR
  * *SDF / multi-channel SDF* for scale/transform-friendly text.
* Packs glyph bitmaps into a dynamic texture atlas (e.g., skyline or guillotine packer).
* Tracks atlas texture regions + LRU eviction when atlas fills; supports multiple atlases if needed.
* Uploads changed atlas regions to GPU with `wgpu::Queue::write_texture` (subresource update).

**C. GPU resources**

* One or more `Texture2D` for atlas (RGBA8 or R8 depending on approach).
* Sampler (linear + clamp).
* Vertex buffer for a unit quad.
* Instance buffer: per-glyph data (position, uv rect, fg color, style flags, timeline metadata).
* Uniform buffer: projection matrix, DPI/scale, time for animations.

**D. Render pipeline & shaders (GPU)**

* Vertex shader: expands instance into screen-space quad vertices (instanced).
* Fragment shader: sample atlas, apply SDF-based smoothstep (if SDF), apply color, gamma-correct, apply effects (cursor glow, selection tint), output to swapchain.
* Optional second pass for compositing terminal effects (blur for overlay, bloom for replay highlights).

**E. Integration with TimeLoop features**

* Timeline → each glyph instance is associated with a `time_created` and optional `time_removed`. The instance buffer can include `appearance_progress` computed from uniform time or computed CPU-side.
* Replay visualization: shader-driven highlight intensity = f(now, time_created, timeline_speed). This keeps replay animations on GPU and cheap.

---

# 3) Glyph atlas design choices (tradeoffs)

**Bitmap atlas (RGBA / single-channel alpha)**

* Simpler, lower CPU work.
* Crisp at native size, but poor when scaling/rotating/zooming.
* Uses `R8` texture if only mask needed (saves memory); RGBA if premultiplied color glyphs (emoji).

**SDF / MSDF**

* Pros: scalable, crisp at many sizes, cheap GPU smoothing.
* Cons: more complex rasterizer and slightly larger memory per glyph. Multi-channel SDF handles sharp corners better (MSDF).
* Recommendation: **MSDF for scalable terminal UI + zoomable replay**. If MVP, start with bitmap; upgrade to SDF when you need crisp zoom.

**Color bitmap fonts (COLR / SBIX / emoji)**

* Keep a separate atlas or layers in the same atlas. Color glyphs should be stored as RGBA; composite in fragment shader.
* Fallback: treat color fonts as bitmaps; no SDF.

---

# 4) Atlas packing & eviction

* Start with a single large atlas (e.g., 4096×4096) and a skyline packer.
* Track `AtlasSlot { x,y,w,h, page_id }` per glyph key `font+size+glyph_id`.
* When full: either create a new atlas page or evict LRU glyphs (but prefer to add pages to avoid re-rasterizing frequently).
* Keep a `generation` counter to know when atlas pages changed and only re-upload changed regions.

---

# 5) Batching, instance format & buffer usage

* Instance struct (compact):

  * `vec2 pos` (x,y in pixels)
  * `vec2 size` (width,height)
  * `vec4 uvRect` (u0,v0,u1,v1) OR `vec2 uvOrigin + vec2 uvSize`
  * `u32 fg_color` (packed rgba8)
  * `u16 flags` (bold/italic/underline/emoji)
  * `f32 time_created` (for replay highlight)
* Use one large `Storage` or `Vertex` buffer with `wgpu::BufferUsages::VERTEX | COPY_DST`.
* Fill CPU-side vector of instances per frame, then `queue.write_buffer` or map buffer once and write.
* Draw with instancing: `draw(0..6, 0..instance_count)` (unit quad 6 verts).

---

# 6) Shaders — conceptual

**Vertex shader**

* Input: unit quad vertex positions ([-0.5..0.5]) + instance data.
* Compute `screen_pos = projection * (instance.pos + vertex_pos * instance.size)`.
* Pass UV computed from instance. Pass `fg_color`, `time_created`.

**Fragment shader**

* Sample atlas at UV.
* If SDF:

  * `alpha = smoothstep(0.5 - aa, 0.5 + aa, sdf_sample)`.
* Else:

  * `alpha = sampled.r` (or sampled.a).
* Compose `out.rgb = mix(bg_color, fg_color, alpha)`.
* Apply selection/cursor highlight blend using `time` uniform and `time_created` to produce animated effect for replay. Use additive or saturation blend for keystroke pulses.

---

# 7) Handling text metrics, subpixel, and hinting

* On macOS and Windows, FreeType hinting improves small-size clarity. Provide a per-OS hinting strategy.
* Subpixel rendering (RGB subpixel) is platform-sensitive; many GPU renderers avoid subpixel unless full control of scanout pipeline is available. For consistent cross-platform output, prefer alpha-based smoothing (SDF).
* DPI / pixel ratio: compute `scale = dpi / 96.0`, rasterize glyphs at requested `em * scale` and store scale in atlas key. When window resizes or device pixel ratio changes, you may need to re-rasterize glyphs for new scale.

---

# 8) Performance considerations & profiling

* Batch everything per-frame. Minimize `draw` calls. Aim for 1-3 draws per frame (background, text, overlays).
* Avoid per-glyph uniform updates — use instance data.
* Use `wgpu` staging buffers or `queue.write_buffer` depending on instance size and frequency. For many small writes, a mapped persistent buffer with `MapMode::Write` may be best.
* Profile with GPU timers (wgpu supports timestamp queries) and CPU timers to find bottlenecks (upload vs shading).
* Keep atlas updates to when new glyphs appear (not every frame). Only `write_texture` changed rectangles.

---

# 9) Edge cases & features you'll want sooner than you expect

* **Ligature highlighting**: when TimeLoop rewinds a command, highlight the full ligature cluster, not just characters. HarfBuzz cluster mapping helps here.
* **UTF-8 multi-column glyphs** (CJK, emoji double-width): covert to cell-occupancy metadata and render combined quads.
* **Selection across shaped runs**: map selection ranges into glyph instances (use HarfBuzz cluster info).
* **Animated cursor + keystroke glow**: GPU time uniform drives smooth animation, so replay looks silky.
* **Search-highlighting/Pinpointing in replay**: render small overlay quads driven by timeline data.
* **Offscreen text caching**: for scrollback far outside viewport, avoid making instance data; only render visible region.

---

# 10) Minimal Rust + wgpu pseudocode sketch (conceptual)

```rust
struct GlyphInstance {
    pos: [f32;2],
    size: [f32;2],
    uv0: [f32;2],
    uv1: [f32;2],
    fg: u32,           // packed rgba
    flags: u16,
    time_created: f32, // seconds
}

fn frame_render(...) {
    // 1) Shape visible lines -> Vec<GlyphPlacement>
    let glyphs = do_shaping(&visible_text);

    // 2) Ensure glyphs in atlas
    for g in glyphs.iter() {
        if !atlas.contains(g.key) {
            let bitmap = freetype_rasterize(g.font, g.glyph_id, size);
            let rect = atlas.insert(bitmap.width, bitmap.height);
            queue.write_texture(..., bitmap.pixels, ...); // upload subrect
            atlas.record(g.key, rect);
        }
    }

    // 3) Build instance buffer
    let mut instances = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        let slot = atlas.lookup(g.key);
        instances.push(GlyphInstance {
            pos: [g.x, g.y],
            size: [slot.w as f32, slot.h as f32],
            uv0: [slot.u0, slot.v0],
            uv1: [slot.u1, slot.v1],
            fg: pack_rgba(g.fg_color),
            flags: g.flags,
            time_created: g.time_created,
        });
    }
    queue.write_buffer(&instance_buffer, 0, bytemuck::cast_slice(&instances));

    // 4) Render pass
    let mut rpass = encoder.begin_render_pass(...);
    rpass.set_pipeline(&text_pipeline);
    rpass.set_bind_group(0, &atlas_bind_group, &[]);
    rpass.set_vertex_buffer(0, &quad_vertex_buffer, 0, 0);
    rpass.set_vertex_buffer(1, &instance_buffer, 0, 0);
    rpass.draw(0..6, 0..instances.len() as u32);
}
```

---

# 11) Why `wgpu` is a strong fit

* Single codebase; it will select Metal on macOS, Vulkan on Linux, and DX12/11 on Windows.
* Safety & Rust ergonomics reduce driver debugging pain.
* Works well with modern rendering patterns (bind groups, explicit resource lifetimes).
* Used by modern editors and apps — proven track record.

---

# 12) Practical roadmap (MVP → production)

1. **MVP (fastest):** bitmap atlas + FreeType + HarfBuzz + single atlas texture + instanced quads + simple alpha sampling fragment shader. Implement with `wgpu`.
2. **V1:** Add dynamic atlas pages, LRU, emoji/color glyph support, selection/cursor effects (GPU-driven). Add smoothing tweaks and DPI awareness.
3. **V2:** Replace bitmap with MSDF for crisp zoom and improved scaling; implement multi-page MSDF atlases. Add shader-based replay visualizations.
4. **Polish:** subpixel options per-platform, font fallback engine, GPU profiling & telemetry, accessibility features (high contrast, scaling).

---

# 13) Additional integration notes for TimeLoop features

* **Recording metadata**: when you record a keystroke/command, store `time_created` and optionally a `visual_tag`. Use these tags to drive shader parameters for replay.
* **Branching timelines**: keep rendering deterministic. Replay must map recorded times to a uniform `replay_time` fed to shaders. This avoids CPU-side re-layout during a replay unless content truly changes.
* **Session replay export**: export atlas pages + instance timeline events; a replay viewer can reconstruct visuals without native fonts by shipping the atlas bitmaps.

---

# 14) Final micro-advice (war stories)

* Start with bitmaps to get the pipeline working end-to-end — shaping, atlas, instancing, shader — then swap the rasterization to SDF.
* Keep the atlas keying deterministic: `font_file_hash + size + glyph_id` so you can cache between runs or snapshot for session exports.
* Test with pathological inputs: long CJK lines, emoji-heavy chat logs, zero-width joiner sequences. HarfBuzz + FreeType combinations catch many nasties but testing saves you from weird ligature bugs mid-demo.
