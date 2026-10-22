@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

struct Params {
    corners: array<vec4<f32>, 4>,
    uvs: array<vec4<f32>, 4>,
    color: vec4<f32>,
}
@group(1) @binding(0) var<uniform> params: Params;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let corner = array(0u, 1u, 2u, 2u, 1u, 3u)[index];
    let uv = params.uvs[corner].xy;
    return Vertex(params.corners[corner], uv);
}

@fragment
fn fs_main(vertex: Vertex) -> @location(0) vec4<f32> {
    let color = textureSample(picture, picture_sampler, vertex.uv);
    let alpha = color.a * params.color.x;
    if alpha <= 0.0 { discard; }
    let rgb = select(color.rgb * color.a, color.rgb, params.color.y > 0.5);
    return vec4(rgb, color.a) * params.color.x;
}
