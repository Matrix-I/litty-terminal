//! `custom-shader`: Ghostty / Shadertoy-style post-processing passes.
//!
//! The cell pass renders into an offscreen texture instead of the window; each
//! shader then runs as a full-screen pass that samples the previous result as
//! `iChannel0` (`mainImage(out vec4 fragColor, in vec2 fragCoord)`), and the
//! last pass draws to the window. Textures are stored bottom row first, so
//! Shadertoy's bottom-left `fragCoord` and `texture(iChannel0, uv)` agree.
//!
//! Animation (`iTime`) means a redraw per frame; it only runs while a shader
//! uses `iTime` / `iFrame` and `custom-shader-animation` allows it.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::render::backend::{CommandEncoder, Device, ObjPtr, Queue, Texture, TextureFormat};

/// Uniforms every pass sees (64 bytes, matches `MSL_PRELUDE`).
#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Globals {
    resolution: [f32; 4],
    mouse: [f32; 4],
    date: [f32; 4],
    time: f32,
    time_delta: f32,
    frame: i32,
    /// 1 when this pass draws to the window (flip `fragCoord` to bottom-left).
    to_screen: u32,
}

const _: () = assert!(std::mem::size_of::<Globals>() == 64);

const MSL_PRELUDE: &str = r#"#include <metal_stdlib>
using namespace metal;

typedef float2 vec2;
typedef float3 vec3;
typedef float4 vec4;
typedef int2 ivec2;
typedef int3 ivec3;
typedef int4 ivec4;
typedef uint2 uvec2;
typedef uint3 uvec3;
typedef uint4 uvec4;
typedef float2x2 mat2;
typedef float3x3 mat3;
typedef float4x4 mat4;

struct LittyGlobals {
    float4 _litty_resolution;
    float4 iMouse;
    float4 iDate;
    float iTime;
    float iTimeDelta;
    int iFrame;
    uint _litty_to_screen;
};

struct LittyChannel {
    texture2d<float> tex;
    sampler smp;
};

static inline float4 texture(LittyChannel ch, float2 uv) {
    return ch.tex.sample(ch.smp, uv);
}

static inline float4 textureLod(LittyChannel ch, float2 uv, float lod) {
    return ch.tex.sample(ch.smp, uv, level(lod));
}

struct PostFxVsOut {
    float4 pos [[position]];
};

vertex PostFxVsOut vs_postfx(uint i [[vertex_id]]) {
    float2 p = float2(float((i << 1u) & 2u), float(i & 2u));
    PostFxVsOut out;
    out.pos = float4(p * 2.0f - 1.0f, 0.0f, 1.0f);
    return out;
}
"#;

/// Wrap a user Shadertoy GLSL snippet into a self-contained MSL shader source,
/// preserving the user snippet between markers for zero-dependency validation.
fn wrap(user: &str) -> String {
    // Transform `out vec4 name` / `in vec2 name` parameter qualifiers for MSL compatibility.
    let msl_body = user
        .replace("out vec4 ", "thread vec4& ")
        .replace("inout vec4 ", "thread vec4& ")
        .replace("in vec2 ", "vec2 ");

    // Inject Shadertoy uniform bindings into `mainImage` scope via macros.
    format!(
        "{MSL_PRELUDE}
/*__LITTY_USER_START__*/
{user}
/*__LITTY_USER_END__*/
#define iResolution (_litty_g._litty_resolution.xyz)
#define iTime (_litty_g.iTime)
#define iTimeDelta (_litty_g.iTimeDelta)
#define iFrame (_litty_g.iFrame)
#define iMouse (_litty_g.iMouse)
#define iDate (_litty_g.iDate)

static LittyGlobals _litty_unused_globals = {{}};
"
    ) + &msl_body.replace(
        "void mainImage(",
        "static inline void _litty_user_mainImage(constant LittyGlobals& _litty_g, LittyChannel iChannel0, ",
    ) + r#"
fragment float4 fs_postfx(
    PostFxVsOut v [[stage_in]],
    constant LittyGlobals& _litty_g [[buffer(0)]],
    texture2d<float> _litty_tex [[texture(0)]],
    sampler _litty_smp [[sampler(0)]]
) {
    float2 fc = v.pos.xy;
    if (_litty_g._litty_to_screen != 0u) {
        fc.y = _litty_g._litty_resolution.y - fc.y;
    }
    float4 color = float4(0.0f);
    LittyChannel ch = { _litty_tex, _litty_smp };
    _litty_user_mainImage(_litty_g, ch, color, fc);
    return color;
}
"#
}

/// Validate a wrapped Shadertoy shader without linking `naga`.
/// Checks balanced braces/parens, `mainImage` entrypoint presence, and Undeclared identifiers.
fn check(src: &str) -> Result<(), String> {
    let user = src
        .split_once("/*__LITTY_USER_START__*/")
        .and_then(|(_, rest)| rest.split_once("/*__LITTY_USER_END__*/"))
        .map_or(src, |(u, _)| u);

    if !user.contains("mainImage") {
        return Err("missing `mainImage` entry point".to_string());
    }

    let mut braces = 0i32;
    let mut parens = 0i32;
    for ch in user.chars() {
        match ch {
            '{' => braces += 1,
            '}' => {
                braces -= 1;
                if braces < 0 {
                    return Err("unmatched `}`".to_string());
                }
            }
            '(' => parens += 1,
            ')' => {
                parens -= 1;
                if parens < 0 {
                    return Err("unmatched `)`".to_string());
                }
            }
            _ => {}
        }
    }
    if braces != 0 || parens != 0 {
        return Err("unbalanced braces or parentheses".to_string());
    }

    let mut known: HashSet<&str> = [
        // Types & qualifiers
        "void", "bool", "int", "uint", "float", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4",
        "uvec2", "uvec3", "uvec4", "bvec2", "bvec3", "bvec4", "mat2", "mat3", "mat4", "in", "out",
        "inout", "const", "struct", "return", "if", "else", "for", "while", "do", "break",
        "continue", "discard", "true", "false", "precision", "highp", "mediump", "lowp",
        // Shadertoy uniforms & entry point
        "mainImage", "iResolution", "iTime", "iTimeDelta", "iFrame", "iMouse", "iDate",
        "iChannel0", "iChannel1", "iChannel2", "iChannel3", "iChannelResolution", "gl_FragCoord",
        // GLSL built-in functions
        "texture", "textureLod", "texelFetch", "textureSize", "sin", "cos", "tan", "asin", "acos",
        "atan", "sinh", "cosh", "tanh", "pow", "exp", "log", "exp2", "log2", "sqrt", "inversesqrt",
        "abs", "sign", "floor", "trunc", "round", "ceil", "fract", "mod", "min", "max", "clamp",
        "mix", "step", "smoothstep", "length", "distance", "dot", "cross", "normalize", "faceforward",
        "reflect", "refract", "matrixCompMult", "lessThan", "lessThanEqual", "greaterThan",
        "greaterThanEqual", "equal", "notEqual", "any", "all", "not", "dFdx", "dFdy", "fwidth",
        "radians", "degrees",
    ]
    .into_iter()
    .collect();

    // Collect identifiers declared after a type name or `#define`.
    let tokens: Vec<(usize, &str)> = scan_ident_tokens(user);
    for (idx, (_, tok)) in tokens.iter().enumerate() {
        if matches!(
            *tok,
            "void" | "bool" | "int" | "uint" | "float" | "vec2" | "vec3" | "vec4"
                | "ivec2" | "ivec3" | "ivec4" | "uvec2" | "uvec3" | "uvec4"
                | "mat2" | "mat3" | "mat4" | "struct"
        ) && let Some((_, next)) = tokens.get(idx + 1)
        {
            known.insert(next);
        }
    }

    // Verify all non-member identifiers are declared.
    let bytes = user.as_bytes();
    for (byte_pos, ident) in tokens {
        // Skip swizzles / struct fields (`c.rgb`, `uv.y`).
        if byte_pos > 0 && bytes[byte_pos - 1] == b'.' {
            continue;
        }
        // Skip float exponent suffixes (`1e-3`, `2.0f`).
        if byte_pos > 0 && (bytes[byte_pos - 1].is_ascii_digit() || bytes[byte_pos - 1] == b'.') {
            continue;
        }
        if !known.contains(ident) {
            return Err(format!("undeclared identifier `{ident}`"));
        }
    }

    Ok(())
}

fn scan_ident_tokens(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip line comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Skip block comments
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // Skip preprocessor lines
        if bytes[i] == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push((start, &s[start..i]));
        } else {
            i += 1;
        }
    }
    out
}

/// Whether the shader changes over time.
fn uses_time(user: &str) -> bool {
    ["iTime", "iFrame", "iTimeDelta", "iDate"].iter().any(|n| user.contains(n))
}

/// A warning printed at most once per process, however many windows or
/// config reloads ask for it.
struct WarnOnce(AtomicBool);

impl WarnOnce {
    const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// `true` for the first caller only.
    fn first(&self) -> bool {
        !self.0.swap(true, Ordering::Relaxed)
    }
}

static UNSUPPORTED_WARNING: WarnOnce = WarnOnce::new();

const UNSUPPORTED: &str = "litty: custom-shader is not supported on Linux yet; the setting is ignored";

/// Read, wrap and check every `custom-shader` file; broken ones are reported
/// and left out. Returns the sources and whether any of them animates.
pub fn load(paths: &[std::path::PathBuf]) -> (Vec<String>, bool) {
    if !supported(paths, crate::render::backend::CUSTOM_SHADERS_SUPPORTED, &UNSUPPORTED_WARNING) {
        return (Vec::new(), false);
    }
    let mut sources = Vec::new();
    let mut animated = false;
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(user) => {
                let src = wrap(&user);
                match check(&src) {
                    Ok(()) => {
                        animated |= uses_time(&user);
                        sources.push(src);
                    }
                    Err(e) => eprintln!("litty: custom-shader {}: {e}", display(path)),
                }
            }
            Err(e) => eprintln!("litty: custom-shader {}: {e}", display(path)),
        }
    }
    (sources, animated)
}

/// Whether `paths` should be loaded. On a backend without custom shaders
/// the key is reported once, rather than as a compile error per shader and
/// window, and nothing is read.
fn supported(paths: &[std::path::PathBuf], backend_supported: bool, warning: &WarnOnce) -> bool {
    if paths.is_empty() || backend_supported {
        return true;
    }
    if warning.first() {
        eprintln!("{UNSUPPORTED}");
    }
    false
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

pub struct PostFx {
    pipelines: Vec<ObjPtr>,
    /// Ping-pong targets; `targets[0]` receives the cell pass.
    targets: Vec<Texture>,
    format: TextureFormat,
    size: (u32, u32),
    start: std::time::Instant,
    last_frame: std::time::Instant,
    frame: i32,
}

impl PostFx {
    /// Build the passes for already checked `sources` (see [`load`]); `None`
    /// when there are none.
    pub fn new(device: &Device, format: TextureFormat, sources: &[String]) -> Option<Self> {
        if sources.is_empty() {
            return None;
        }
        let mut pipelines = Vec::with_capacity(sources.len());
        for src in sources {
            match device.create_postfx_pipeline(src, format) {
                Ok(p) => pipelines.push(p),
                Err(e) => eprintln!("litty: custom-shader compile error: {e}"),
            }
        }
        if pipelines.is_empty() {
            return None;
        }
        let now = std::time::Instant::now();
        Some(PostFx {
            pipelines,
            targets: Vec::new(),
            format,
            size: (0, 0),
            start: now,
            last_frame: now,
            frame: 0,
        })
    }

    /// Discard offscreen ping-pong render targets to release VRAM when window shrinks.
    pub fn shrink(&mut self) {
        self.targets.clear();
        self.size = (0, 0);
    }

    /// The texture the cell pass draws into, (re)created at the window size.
    pub fn cell_target(&mut self, device: &Device, width: u32, height: u32) -> &Texture {
        let wanted = self.pipelines.len().min(2);
        if self.size != (width, height) || self.targets.len() != wanted {
            self.size = (width, height);
            self.targets = (0..wanted)
                .map(|_| Texture::new(device, width.max(1), height.max(1), self.format, true))
                .collect();
        }
        &self.targets[0]
    }

    /// Run every pass after the cell pass, ending on `screen_target`.
    pub fn run(
        &mut self,
        device: &Device,
        _queue: &Queue,
        encoder: &mut CommandEncoder,
        screen_target: crate::render::backend::SurfaceFrame,
        mouse: (f32, f32),
    ) -> crate::render::backend::SurfaceFrame {
        let now = std::time::Instant::now();
        let (w, h) = self.size;
        let date = date_now();
        let mut globals = Globals {
            resolution: [w as f32, h as f32, 1.0, 0.0],
            mouse: [mouse.0, h as f32 - mouse.1, 0.0, 0.0],
            date,
            time: (now - self.start).as_secs_f32(),
            time_delta: (now - self.last_frame).as_secs_f32(),
            frame: self.frame,
            to_screen: 0,
        };
        self.last_frame = now;
        self.frame = self.frame.wrapping_add(1);

        let sampler = device.linear_sampler();
        let last = self.pipelines.len() - 1;
        for (i, pipeline) in self.pipelines.iter().enumerate() {
            globals.to_screen = (i == last) as u32;
            let dest_ptr = if i == last {
                screen_target.texture_ptr()
            } else {
                self.targets[(i + 1) % 2].raw_ptr()
            };
            let source = &self.targets[i % 2];
            let mut pass = encoder.begin_render_pass(dest_ptr, Some([0.0, 0.0, 0.0, 1.0]));
            pass.draw_postfx(pipeline, &globals, source, sampler);
        }
        screen_target
    }
}

/// Shadertoy's `iDate`: year, month (0-based), day, seconds since midnight (UTC).
fn date_now() -> [f32; 4] {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [year as f32, (month - 1) as f32, day as f32, (secs % 86_400) as f32]
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsupported_custom_shaders_are_skipped_and_reported_once() {
        let warning = super::WarnOnce::new();
        let paths = [std::path::PathBuf::from("/nonexistent/crt.glsl")];
        assert!(!super::supported(&paths, false, &warning), "first window");
        assert!(!warning.first(), "the first call printed the warning");
        assert!(!super::supported(&paths, false, &warning), "second window: skipped, not printed again");
        assert!(super::supported(&paths, true, &super::WarnOnce::new()), "Metal loads them");
        assert!(super::supported(&[], false, &super::WarnOnce::new()), "nothing configured, nothing to say");
    }

    #[test]
    fn a_warning_is_due_only_once() {
        let warning = super::WarnOnce::new();
        assert!(warning.first());
        assert!(!warning.first());
        assert!(!warning.first());
    }

    use super::*;

    #[test]
    fn shadertoy_shaders_compile() {
        let invert = "void mainImage(out vec4 fragColor, in vec2 fragCoord) {
            vec2 uv = fragCoord / iResolution.xy;
            vec4 c = texture(iChannel0, uv);
            fragColor = vec4(1.0 - c.rgb, c.a);
        }";
        check(&wrap(invert)).unwrap();
        assert!(!uses_time(invert));

        let crt = "void mainImage(out vec4 fragColor, in vec2 fragCoord) {
            vec2 uv = fragCoord.xy / iResolution.xy;
            float scan = 0.9 + 0.1 * sin(uv.y * iResolution.y * 3.14159 + iTime);
            fragColor = texture(iChannel0, uv) * scan;
        }";
        check(&wrap(crt)).unwrap();
        assert!(uses_time(crt));

        assert!(check(&wrap("void mainImage(out vec4 c, in vec2 f) { c = nope; }")).is_err());
    }

    #[test]
    fn date_is_plausible() {
        let [year, month, day, secs] = date_now();
        assert!(year >= 2024.0 && (0.0..12.0).contains(&month) && (1.0..=31.0).contains(&day));
        assert!((0.0..86_400.0).contains(&secs));
    }
}
