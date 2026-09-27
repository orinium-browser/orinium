struct Uniforms {
    screen_size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var atlas: texture_2d_array<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) layer: f32,
    @location(2) color: vec4<f32>,
};

fn unpack_i16_hi(v: u32) -> f32 {
    return f32(i32(v) >> 16);
}
fn unpack_i16_lo(v: u32) -> f32 {
    let shifted = v << 16u;
    return f32(i32(shifted) >> 16);
}
fn unpack_u16_hi(v: u32) -> f32 {
    return f32(v >> 16u);
}
fn unpack_u16_lo(v: u32) -> f32 {
    return f32(v & 0xFFFFu);
}

@vertex
fn vs_main(
    @location(0) quad_pos: vec2<f32>,
    @location(1) quad_uv: vec2<f32>,
    @location(2) raw_pos: vec2<f32>,
    @location(3) raw_size: u32,
    @location(4) raw_uv_off: u32,
    @location(5) raw_uv_size: u32,
    @location(6) raw_layer: u32,
    @location(7) raw_color: u32,
) -> VertexOutput {
    let gx_px = raw_pos.x + (quad_pos.x * unpack_u16_hi(raw_size)) / 64.0;
    let gy_px = raw_pos.y + (quad_pos.y * unpack_u16_lo(raw_size)) / 64.0;

    let ndc_x = (gx_px / uniforms.screen_size.x) * 2.0 - 1.0;
    let ndc_y = -((gy_px / uniforms.screen_size.y) * 2.0 - 1.0);

    let u_off = unpack_i16_hi(raw_uv_off) / 2048.0;
    let v_off = unpack_i16_lo(raw_uv_off) / 2048.0;
    let uv_w = unpack_u16_hi(raw_uv_size) / 2048.0;
    let uv_h = unpack_u16_lo(raw_uv_size) / 2048.0;

    let tex_coord = vec2<f32>(u_off, v_off) + quad_uv * vec2<f32>(uv_w, uv_h);
    let layer_f = unpack_u16_lo(raw_layer);

    let r = f32((raw_color >> 24u) & 0xFFu) / 255.0;
    let g = f32((raw_color >> 16u) & 0xFFu) / 255.0;
    let b = f32((raw_color >> 8u) & 0xFFu) / 255.0;
    let a = f32(raw_color & 0xFFu) / 255.0;

    var out: VertexOutput;
    out.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.tex_coord = tex_coord;
    out.layer = layer_f;
    out.color = vec4<f32>(r, g, b, a);
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let layer = i32(input.layer);
    let alpha = textureSample(atlas, atlas_sampler, input.tex_coord, layer).r;
    return vec4<f32>(input.color.rgb, input.color.a * alpha);
}
