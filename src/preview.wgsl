@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
// alpha, premultiplied input, invert RGB, padding
@group(1) @binding(0) var<uniform> params: vec4<f32>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let uv = array(vec2(0.0, 0.0), vec2(2.0, 0.0), vec2(0.0, 2.0))[index];
    return Vertex(vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

@fragment
fn fs_main(vertex: Vertex) -> @location(0) vec4<f32> {
    let color = textureSample(picture, picture_sampler, vertex.uv);
    var rgb = select(color.rgb * color.a, color.rgb, params.y > 0.5);
    if params.z > 0.5 {
        rgb = vec3(color.a) - rgb;
    }
    return vec4(rgb, color.a) * params.x;
}
