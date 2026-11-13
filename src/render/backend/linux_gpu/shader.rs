//! `custom-shader` on Vulkan: the GLSL that `postfx` wraps around a user
//! shader is compiled to SPIR-V with naga when the config loads (Vulkan
//! takes no shader source), then built into a full-screen pass pipeline.

use naga::back::spv;
use naga::front::glsl;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{ShaderStage, Span};

use super::device::{shared_pipelines, Blend, Device};
use super::ffi::*;
use crate::render::backend::TextureFormat;

/// One triangle covering the target, from the vertex index alone (as
/// `vs_postfx` on Metal); the pass draws 3 vertices without buffers.
const VERTEX_GLSL: &str = "#version 450
void main() {
    vec2 p = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Compile GLSL 450 for `stage` to SPIR-V 1.0 words. Errors name the line
/// counted from `first_line` (the first line of the user's file within
/// `src`), so they point into the file the user wrote.
fn compile(src: &str, stage: ShaderStage, first_line: u32) -> Result<Vec<u32>, String> {
    let at = |span: Span| {
        if !span.is_defined() {
            return String::new();
        }
        let line = span.location(src).line_number;
        match line.checked_sub(first_line) {
            Some(n) => format!("line {}: ", n + 1),
            None => "in the code litty adds: ".to_string(),
        }
    };
    let module = glsl::Frontend::default().parse(&glsl::Options::from(stage), src).map_err(|e| {
        e.errors.iter().map(|e| format!("{}{}", at(e.meta), e.kind)).collect::<Vec<_>>().join("; ")
    })?;
    // Push constants (`layout(push_constant)`) are naga's "immediates".
    let info = Validator::new(ValidationFlags::all(), Capabilities::IMMEDIATES).validate(&module).map_err(|e| {
        let span = e.spans().next().map_or_else(Span::default, |(span, _)| *span);
        format!("{}{}", at(span), e.as_inner())
    })?;
    // Plain Vulkan conventions (no wgpu coordinate flip), no debug names.
    let options = spv::Options { flags: spv::WriterFlags::empty(), ..spv::Options::default() };
    spv::write_vec(&module, &info, &options, None).map_err(|e| e.to_string())
}

/// 1-based line of `src` where the user's shader starts: the line after
/// `postfx`'s start marker, else the first.
fn user_first_line(src: &str) -> u32 {
    src.find(crate::render::postfx::USER_START)
        .map_or(1, |at| src[..at].matches('\n').count() as u32 + 2)
}

impl Device {
    /// Build a `custom-shader` pass from `glsl_src` (`postfx::wrap`). The
    /// pipeline uses the shared swapchain pass, which offscreen passes of
    /// the same format are compatible with.
    pub fn create_postfx_pipeline(&self, glsl_src: &str, format: TextureFormat) -> Result<ObjPtr, String> {
        let fragment = compile(glsl_src, ShaderStage::Fragment, user_first_line(glsl_src))?;
        let vertex = compile(VERTEX_GLSL, ShaderStage::Vertex, 1)?;
        let Some(vk) = &self.vk else { return Err("no Vulkan device".into()) };
        let render_pass = shared_pipelines(self, format).render_pass_clear;
        if render_pass == 0 {
            return Err("no render pass for the window's format".into());
        }
        // SAFETY: `vk.device` is live; the modules are destroyed once the
        // pipeline holds what it needs from them.
        unsafe {
            let create = |words: &[u32]| super::device::create_shader_module(&vk.lib, vk.device, bytemuck::cast_slice(words));
            let (vs, fs) = (create(&vertex), create(&fragment));
            let pipeline = if vs != 0 && fs != 0 {
                super::device::create_graphics_pipeline(
                    &vk.lib,
                    vk.device,
                    render_pass,
                    vk.postfx_pipe_layout,
                    (vs, c"main"),
                    (fs, c"main"),
                    0,
                    &[],
                    Blend::Replace,
                )
            } else {
                0
            };
            for module in [vs, fs].into_iter().filter(|&m| m != 0) {
                (vk.lib.vkDestroyShaderModule)(vk.device, module, std::ptr::null());
            }
            if pipeline == 0 {
                return Err("the driver could not build the pipeline".into());
            }
            Ok(ObjPtr(pipeline))
        }
    }

    /// Release a pipeline from `create_postfx_pipeline` (config reload or a
    /// closed window). Waits for the GPU, since a frame may still use it.
    pub fn destroy_postfx_pipeline(&self, pipeline: &ObjPtr) {
        let Some(vk) = &self.vk else { return };
        if pipeline.0 == 0 {
            return;
        }
        // SAFETY: once the device is idle no recorded frame refers to it.
        unsafe {
            (vk.lib.vkDeviceWaitIdle)(vk.device);
            (vk.lib.vkDestroyPipeline)(vk.device, pipeline.0, std::ptr::null());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::postfx;

    const INVERT: &str = "void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec4 c = texture(iChannel0, uv);
    fragColor = vec4(1.0 - c.rgb, c.a);
}";

    /// SPIR-V starts with its magic number; each Shadertoy uniform and
    /// helper the wrapper offers compiles.
    #[test]
    fn shadertoy_shaders_compile_to_spirv() {
        let words = compile(&postfx::wrap(INVERT), ShaderStage::Fragment, 1).unwrap();
        assert_eq!(words[0], 0x0723_0203);
        let everything = "void mainImage(out vec4 o, in vec2 f) {
    vec2 uv = f / iResolution.xy + iMouse.xy * 0.0;
    float t = iTime + iTimeDelta + float(iFrame) + iDate.w;
    o = textureLod(iChannel0, uv, 0.0) * (0.5 + 0.5 * sin(t));
}";
        compile(&postfx::wrap(everything), ShaderStage::Fragment, 1).unwrap();
        assert_eq!(compile(VERTEX_GLSL, ShaderStage::Vertex, 1).unwrap()[0], 0x0723_0203);
    }

    /// A mistake is reported at its line in the user's file, not in the
    /// wrapped source.
    #[test]
    fn errors_point_into_the_users_file() {
        let src = postfx::wrap("void mainImage(out vec4 o, in vec2 f) {\n    o = vec4(1.0);\n    o = nope;\n}");
        let err = compile(&src, ShaderStage::Fragment, user_first_line(&src)).unwrap_err();
        assert!(err.starts_with("line 3: "), "{err}");
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn a_shader_without_main_image_is_an_error() {
        let src = postfx::wrap("void notMain(out vec4 o, in vec2 f) { o = vec4(1.0); }");
        assert!(compile(&src, ShaderStage::Fragment, user_first_line(&src)).is_err());
    }

    #[test]
    fn user_lines_start_after_the_marker() {
        assert_eq!(user_first_line("no marker"), 1);
        let src = format!("a\nb\n{}\nuser", postfx::USER_START);
        assert_eq!(user_first_line(&src), 4);
    }

    /// The pipeline is built against the shared swapchain pass and works in
    /// offscreen passes; it is released again without errors.
    #[test]
    fn a_compiled_shader_becomes_a_pipeline() {
        let _gpu = crate::render::backend::TEST_GPU.lock();
        let (device, _queue) = super::super::device::shared_device_and_queue();
        let result = device.create_postfx_pipeline(&postfx::wrap(INVERT), TextureFormat::Bgra8Unorm);
        if device.vk.is_none() {
            assert_eq!(result.err().as_deref(), Some("no Vulkan device"));
            return;
        }
        let pipeline = result.unwrap();
        assert_ne!(pipeline.0, 0);
        device.destroy_postfx_pipeline(&pipeline);
    }

    /// Run `user` as one pass over a `source` cleared to `clear`, with
    /// `to_screen` set as for the last pass, and read back the 8x4 result
    /// (BGRA rows, top first).
    fn shade(user: &str, clear: [f64; 4], to_screen: bool) -> Option<Vec<u8>> {
        use super::super::pass::CommandEncoder;
        use super::super::resources::Texture;
        let _gpu = crate::render::backend::TEST_GPU.lock();
        let (device, queue) = super::super::device::shared_device_and_queue();
        if device.vk.is_none() {
            eprintln!("no GPU; skipping");
            return None;
        }
        let format = TextureFormat::Bgra8Unorm;
        let (w, h) = (8u32, 4u32);
        let pipeline = device.create_postfx_pipeline(&postfx::wrap(user), format).unwrap();
        let source = Texture::new(&device, w, h, format, true);
        let target = Texture::new(&device, w, h, format, true);
        let mut encoder = CommandEncoder::new(&queue);
        drop(encoder.begin_render_pass(source.raw_ptr(), Some(clear)));
        {
            // `postfx::Globals`: resolution, mouse, date, then time, delta,
            // frame and the to-screen flag.
            let mut globals = [0u32; 16];
            globals[0] = (w as f32).to_bits();
            globals[1] = (h as f32).to_bits();
            globals[15] = u32::from(to_screen);
            let mut pass = encoder.begin_render_pass(target.raw_ptr(), Some([0.0, 0.0, 0.0, 1.0]));
            pass.draw_postfx(&pipeline, &globals, &source, device.linear_sampler());
        }
        encoder.submit_and_wait();
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        target.read_pixels(&mut pixels, w * 4);
        drop((source, target));
        device.destroy_postfx_pipeline(&pipeline);
        Some(pixels)
    }

    /// The pass samples the previous one through `iChannel0` and writes
    /// what `mainImage` returns.
    #[test]
    fn a_pass_shades_its_source_through_the_users_shader() {
        let Some(pixels) = shade(INVERT, [0.2, 0.4, 0.6, 1.0], false) else { return };
        for px in pixels.as_chunks::<4>().0 {
            let expected = [102, 153, 204, 255]; // BGRA of 1 - (0.2, 0.4, 0.6)
            assert!(px.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1), "{px:?}");
        }
    }

    /// `fragCoord` is bottom-left on the last pass (it draws to the window
    /// top row first) and top-left in between, where every texture is
    /// stored bottom row first; same as on Metal.
    #[test]
    fn frag_coord_starts_at_the_bottom_on_the_last_pass() {
        let rows = "void mainImage(out vec4 o, in vec2 f) { o = vec4(f.y / iResolution.y, 0.0, 0.0, 1.0); }";
        let red_of_row = |pixels: &[u8], row: usize| pixels[row * 8 * 4 + 2];
        let Some(between) = shade(rows, [0.0; 4], false) else { return };
        let Some(last) = shade(rows, [0.0; 4], true) else { return };
        assert!(red_of_row(&between, 0) < red_of_row(&between, 3), "row 0 is fragCoord.y = 0.5");
        assert!(red_of_row(&last, 0) > red_of_row(&last, 3), "row 0 is the top: fragCoord.y = 3.5");
    }

    #[test]
    fn compile_errors_come_before_any_gpu_work() {
        let device = Device { vk: None };
        let err = device.create_postfx_pipeline(&postfx::wrap("void mainImage(out vec4 o, in vec2 f) { o = x; }"), TextureFormat::Bgra8Unorm);
        let err = err.err().expect("a compile error");
        assert!(err.starts_with("line 1: "), "{err}");
    }
}
