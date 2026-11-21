// Premultiply pass: drawn over a finished translucent frame with
// `vs_corner` (shader.wgsl). The blend state multiplies each pixel's color
// by its alpha, because compositors read the frame as premultiplied; this
// output only feeds factors that ignore it.
//
// premultiply.spv is generated from this file with naga 30
// (`wgsl-in` + `spv-out`, SPIR-V 1.0, no writer flags).

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
