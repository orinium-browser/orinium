struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coord: vec2<f32>,
    @location(2) layer: f32,
    @location(3) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) layer: f32,
    @location(2) color: vec4<f32>,
};

@group(0) @binding(0) var atlas: texture_2d_array<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_pos = vec4<f32>(input.position, 0.0, 1.0);
    output.tex_coord = input.tex_coord;
    output.layer = input.layer;
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let layer = i32(input.layer);
    let alpha = textureSample(atlas, atlas_sampler, input.tex_coord, layer).r;
    return vec4<f32>(input.color.rgb, input.color.a * alpha);
}
