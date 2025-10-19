struct TextVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct TextVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) v_uv: vec2<f32>,
    @location(1) v_color: vec4<f32>,
}

@vertex
fn vs_text(in: TextVertexInput) -> TextVertexOutput {
    var out: TextVertexOutput;
    out.clip_position = vec4<f32>(in.position, 1.0);
    out.v_uv = in.uv;
    out.v_color = in.color;
    return out;
}

@group(0) @binding(0) var font_texture: texture_2d<f32>;
@group(0) @binding(1) var font_sampler: sampler;

@fragment
fn fs_text(in: TextVertexOutput) -> @location(0) vec4<f32> {
    let example = textureSample(font_texture, font_sampler, in.v_uv);
    let alpha = example.r;
    return vec4<f32>(in.v_color.rgb, in.v_color.a * alpha);
}