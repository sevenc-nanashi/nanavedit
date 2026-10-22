// alpha, premultiplied input, padding
@group(1) @binding(0) var<uniform> params: vec4<f32>;

@fragment
fn fs_main(vertex: Vertex) -> @location(0) vec4<f32> {
    let color = textureSample(picture, picture_sampler, vertex.uv);
    let rgb = select(color.rgb * color.a, color.rgb, params.y > 0.5);
    return vec4(rgb, color.a) * params.x;
}
