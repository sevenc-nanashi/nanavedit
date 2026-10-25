struct Params { width: u32, height: u32, stride: u32, kind: u32, bottom_up: u32, pad0: u32, pad1: u32, pad2: u32 }
@group(0) @binding(0) var<storage, read> data: array<u32>;
@group(0) @binding(1) var output: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: Params;
fn byte(offset: u32) -> u32 { return (data[offset / 4u] >> ((offset % 4u) * 8u)) & 255u; }
fn word(offset: u32) -> u32 { return byte(offset) | (byte(offset + 1u) << 8u); }
fn signed_word(offset: u32) -> f32 { return f32(bitcast<i32>(word(offset) << 16u) >> 16); }
fn linear(value: vec3<f32>) -> vec3<f32> {
    return select(value / 12.92, pow(max((value + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4)), value > vec3<f32>(0.04045));
}
fn yuv(y: f32, u: f32, v: f32) -> vec3<f32> { return vec3<f32>(y + 1.402 * v, y - 0.344136 * u - 0.714136 * v, y + 1.772 * u); }
@compute @workgroup_size(8, 8)
fn convert(@builtin(global_invocation_id) pos: vec3<u32>) {
    if pos.x >= params.width || pos.y >= params.height { return; }
    let row = select(pos.y, params.height - 1u - pos.y, params.bottom_up != 0u) * params.stride;
    var rgb = vec3<f32>(0.0); var alpha = 1.0;
    switch params.kind {
        case 0u, 1u: {
            let offset = row + pos.x * select(3u, 4u, params.kind == 1u);
            rgb = vec3<f32>(f32(byte(offset + 2u)), f32(byte(offset + 1u)), f32(byte(offset))) / 255.0;
            if params.kind == 1u { alpha = f32(byte(offset + 3u)) / 255.0; }
        }
        case 2u: {
            let offset = row + (pos.x / 2u) * 4u;
            rgb = yuv((f32(byte(offset + (pos.x % 2u) * 2u)) - 16.0) / 219.0,
                (f32(byte(offset + 1u)) - 128.0) / 224.0, (f32(byte(offset + 3u)) - 128.0) / 224.0);
        }
        case 3u, 4u: {
            let offset = row + pos.x * 8u;
            var rgba: vec4<f32>;
            if params.kind == 3u {
                rgba = vec4<f32>(f32(word(offset)), f32(word(offset+2u)), f32(word(offset+4u)), f32(word(offset+6u))) / 65535.0;
            } else {
                rgba = vec4<f32>(unpack2x16float(data[offset/4u]), unpack2x16float(data[offset/4u+1u]));
            }
            alpha = clamp(rgba.a, 0.0, 1.0);
            if alpha > 0.0 { rgb = rgba.rgb / alpha; }
        }
        case 5u: {
            let offset = row + pos.x * 6u;
            rgb = yuv(signed_word(offset)/4096.0, signed_word(offset+2u)/4096.0, signed_word(offset+4u)/4096.0);
        }
        default: {}
    }
    textureStore(output, pos.xy, vec4<f32>(linear(rgb) * alpha, alpha));
}
