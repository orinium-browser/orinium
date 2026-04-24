struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) params: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) params: vec3<f32>,
}

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(model.position, 1.0);
    out.color = model.color;
    out.params = model.params;
    return out;
}

fn rounded_box_sdf(p: vec2<f32>, r: f32) -> f32 {
    let center = vec2<f32>(0.5, 0.5);
    let half = vec2<f32>(0.5 - r, 0.5 - r);
    let q = abs(p - center) - half;
    let max_q = max(q, vec2<f32>(0.0, 0.0));
    return length(max_q) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let col = in.color;
    let nx = in.params.x;
    let ny = in.params.y;
    let rnorm = in.params.z;

    if (rnorm <= 0.0) {
        return col;
    }

    let p = vec2<f32>(nx, ny);
    let d = rounded_box_sdf(p, rnorm);
    let aa = 1.0 / 128.0;
    let alpha = clamp(1.0 - d / aa, 0.0, 1.0);
    return vec4<f32>(col.rgb, col.a * alpha);
}
