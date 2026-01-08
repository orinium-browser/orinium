struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) in_pos: vec2<f32>,
    @location(1) in_uv: vec2<f32>
) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in_pos, 0.0, 1.0);
    out.uv = in_uv;
    return out;
}

@group(0) @binding(0)
var texture_tex: texture_2d<f32>;

@group(0) @binding(1)
var texture_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color: vec4<f32> = textureSample(texture_tex, texture_sampler, in.uv);
    return color;
}
