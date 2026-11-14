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

#[cfg(any(target_os = "macos", test))]
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::render::backend::{CommandEncoder, Device, ObjPtr, Queue, Texture, TextureFormat};

/// Marks where the user's shader starts in the wrapped source.
pub(crate) const USER_START: &str = "/*__LITTY_USER_START__*/";
pub(crate) const USER_END: &str = "/*__LITTY_USER_END__*/";

/// Uniforms every pass sees (64 bytes, matches `MSL_PRELUDE` and
/// `GLSL_PRELUDE`).
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

#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
fn wrap(user: &str) -> String {
    // Transform `out vec4 name` / `in vec2 name` parameter qualifiers for MSL compatibility.
    let msl_body = user
        .replace("out vec4 ", "thread vec4& ")
        .replace("inout vec4 ", "thread vec4& ")
        .replace("in vec2 ", "vec2 ");

    // Inject Shadertoy uniform bindings into `mainImage` scope via macros.
    format!(
        "{MSL_PRELUDE}
{USER_START}
{user}
{USER_END}
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

/// Uniforms and `iChannel0` for GLSL 450 on Vulkan. The uniforms are push
/// constants (one block per pass); the source is a render target sampled
/// linearly. Members are not named like the Shadertoy macros, which would
/// otherwise expand inside their own definitions.
#[cfg(not(target_os = "macos"))]
const GLSL_PRELUDE: &str = "#version 450
layout(push_constant) uniform LittyGlobals {
    vec4 resolution;
    vec4 mouse;
    vec4 date;
    float time;
    float time_delta;
    int frame;
    uint to_screen;
} _litty_g;
layout(set = 0, binding = 1) uniform texture2D _litty_tex;
layout(set = 0, binding = 2) uniform sampler _litty_smp;
layout(location = 0) out vec4 _litty_color;
#define iResolution (_litty_g.resolution.xyz)
#define iMouse (_litty_g.mouse)
#define iDate (_litty_g.date)
#define iTime (_litty_g.time)
#define iTimeDelta (_litty_g.time_delta)
#define iFrame (_litty_g.frame)
#define iChannel0 sampler2D(_litty_tex, _litty_smp)
";

/// Wrap a user Shadertoy GLSL snippet into a GLSL 450 fragment shader. The
/// user's code is kept as written (Shadertoy's `mainImage` is valid GLSL);
/// `main` flips `fragCoord` to bottom-left for the last pass, as on Metal
/// (Vulkan's `gl_FragCoord` starts top-left too).
#[cfg(not(target_os = "macos"))]
pub(crate) fn wrap(user: &str) -> String {
    format!(
        "{GLSL_PRELUDE}{USER_START}
{user}
{USER_END}
void main() {{
    vec2 fc = gl_FragCoord.xy;
    if (_litty_g.to_screen != 0u) {{
        fc.y = _litty_g.resolution.y - fc.y;
    }}
    vec4 color = vec4(0.0);
    mainImage(color, fc);
    _litty_color = color;
}}
"
    )
}

/// Validate a wrapped shader when the config loads. Metal compiles only when
/// a window builds the pipeline, so a heuristic check runs first there;
/// Vulkan compiles here, which reports real errors with their line.
fn validate(src: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    check(src)?;
    // A missing `}` makes the compiler fail in the wrapper after the user's
    // code; say where the mistake probably is.
    crate::render::backend::validate_postfx(src)
        .map_err(|e| match balance(user_code(src)) {
            Ok(()) => e,
            Err(hint) => format!("{e} ({hint} in your shader?)"),
        })
}

/// The user's shader within a wrapped source.
fn user_code(src: &str) -> &str {
    src.split_once(USER_START)
        .and_then(|(_, rest)| rest.split_once(USER_END))
        .map_or(src, |(u, _)| u)
}

/// Whether braces and parentheses pair up (comments are not skipped).
fn balance(user: &str) -> Result<(), String> {
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
    Ok(())
}

/// Validate a wrapped Shadertoy shader before the backend compiles it.
/// Checks balanced braces/parens, `mainImage` entrypoint presence, and Undeclared identifiers.
#[cfg(any(target_os = "macos", test))]
fn check(src: &str) -> Result<(), String> {
    let user = user_code(src);
    if !user.contains("mainImage") {
        return Err("missing `mainImage` entry point".to_string());
    }
    balance(user).map_err(|error| format!("{error} in your shader?"))?;

    let mut known: HashSet<&str> = [
        // Types & qualifiers
        "void", "bool", "int", "uint", "float", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4",
        "uvec2", "uvec3", "uvec4", "bvec2", "bvec3", "bvec4", "mat2", "mat3", "mat4", "in", "out",
        "inout", "const", "struct", "return", "if", "else", "for", "while", "do", "break",
        "continue", "discard", "true", "false", "precision", "highp", "mediump", "lowp",
        // Shadertoy uniforms & entry point
        "mainImage", "iResolution", "iTime", "iTimeDelta", "iFrame", "iMouse", "iDate",
        "iChannel0", "gl_FragCoord",
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
    // `#define NAME ...` (the scan below skips preprocessor lines).
    known.extend(user.lines().filter_map(|line| {
        let name = line.trim_start().strip_prefix('#')?.trim_start().strip_prefix("define")?.trim_start();
        name.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().filter(|n| !n.is_empty())
    }));

    // Collect identifiers declared after a type name.
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

#[cfg(any(target_os = "macos", test))]
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

/// A `custom-shader` file, wrapped for the backend.
pub struct Source {
    path: PathBuf,
    src: String,
}

/// Read, wrap and validate every `custom-shader` file; broken ones are left
/// out and added to `problems` (see [`report`]). Returns the sources and
/// whether any of them animates.
pub fn load(paths: &[PathBuf], problems: &mut Vec<String>) -> (Vec<Source>, bool) {
    let mut sources = Vec::new();
    let mut animated = false;
    for path in paths {
        let loaded = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|user| {
            let src = wrap(&user);
            validate(&src).map(|()| (user, src))
        });
        match loaded {
            Ok((user, src)) => {
                animated |= uses_time(&user);
                sources.push(Source { path: path.clone(), src });
            }
            Err(e) => problems.push(problem(path, &e)),
        }
    }
    (sources, animated)
}

fn problem(path: &Path, error: &str) -> String {
    format!("litty: custom-shader {}: {error}", path.display())
}

/// Print `problems` on stderr unless they are the ones printed last: every
/// window loads the shaders, and again on each config reload.
pub fn report(problems: &[String]) {
    static LAST: Mutex<Option<u64>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if fresh(&mut last, problems) {
        for p in problems {
            eprintln!("{p}");
        }
    }
}

/// Whether `problems` differ from those `last` remembers (a hash of them),
/// remembering them. No problems forget the last ones, so a shader that
/// breaks again in the same way is reported again.
fn fresh(last: &mut Option<u64>, problems: &[String]) -> bool {
    if problems.is_empty() {
        *last = None;
        return false;
    }
    let mut hasher = std::hash::DefaultHasher::new();
    problems.hash(&mut hasher);
    let hash = Some(hasher.finish());
    let fresh = *last != hash;
    *last = hash;
    fresh
}

pub struct PostFx {
    /// Releases `pipelines` on drop (a config reload replaces them).
    device: Device,
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
    /// Build the passes for already validated `sources` (see [`load`]);
    /// shaders the backend still rejects go to `problems`. `None` when no
    /// pass is left.
    pub fn new(device: &Device, format: TextureFormat, sources: &[Source], problems: &mut Vec<String>) -> Option<Self> {
        if sources.is_empty() {
            return None;
        }
        let mut pipelines = Vec::with_capacity(sources.len());
        for source in sources {
            match device.create_postfx_pipeline(&source.src, format) {
                Ok(p) => pipelines.push(p),
                Err(e) => problems.push(problem(&source.path, &e)),
            }
        }
        if pipelines.is_empty() {
            return None;
        }
        let now = std::time::Instant::now();
        Some(PostFx {
            device: device.clone(),
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

impl Drop for PostFx {
    fn drop(&mut self) {
        for pipeline in &self.pipelines {
            self.device.destroy_postfx_pipeline(pipeline);
        }
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
    fn defined_names_are_declared() {
        check(&wrap("#define PI 3.14159\nvoid mainImage(out vec4 o, in vec2 f) { o = vec4(1.0); o.x = PI; }")).unwrap();
    }

    /// Only `iChannel0` is bound; the others would fail later, in the
    /// backend's compiler, without naming the identifier.
    #[test]
    fn channels_litty_does_not_bind_are_undeclared() {
        let err = check(&wrap("void mainImage(out vec4 o, in vec2 f) { o = texture(iChannel1, f); }")).unwrap_err();
        assert!(err.contains("iChannel1"), "{err}");
    }

    /// Each problem names its file, and the good files still load.
    #[test]
    fn problems_name_the_file() {
        let dir = std::env::temp_dir().join(format!("litty-postfx-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (good, bad, missing) = (dir.join("good.glsl"), dir.join("bad.glsl"), dir.join("missing.glsl"));
        std::fs::write(&good, "#define K 0.5\nvoid mainImage(out vec4 o, in vec2 f) { o = vec4(K); }").unwrap();
        std::fs::write(&bad, "void mainImage(out vec4 o, in vec2 f) {\n o = nope;\n}").unwrap();
        let mut problems = Vec::new();
        let (sources, animated) = load(&[good.clone(), bad.clone(), missing.clone()], &mut problems);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(sources.iter().map(|s| &s.path).collect::<Vec<_>>(), [&good]);
        assert!(!animated);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains(&*bad.to_string_lossy()) && problems[0].contains("nope"), "{problems:?}");
        assert!(problems[1].contains(&*missing.to_string_lossy()), "{problems:?}");
    }

    /// The compiler sees a missing `}` only in the wrapper after the user's
    /// code; the message says the user's shader is the likely place.
    #[test]
    fn a_missing_brace_points_at_the_users_shader() {
        let err = validate(&wrap("void mainImage(out vec4 o, in vec2 f) {\n o = vec4(1.0);\n")).unwrap_err();
        assert!(err.contains("unbalanced braces or parentheses in your shader?"), "{err}");
    }

    /// Every window loads the shaders (and again on each config reload);
    /// the same problems are printed once, and again after they were fixed.
    #[test]
    fn the_same_problems_are_reported_once() {
        let mut last = None;
        let (a, b) = (vec!["a".to_string()], vec!["b".to_string()]);
        assert!(fresh(&mut last, &a));
        assert!(!fresh(&mut last, &a));
        assert!(fresh(&mut last, &b));
        assert!(!fresh(&mut last, &[]));
        assert!(fresh(&mut last, &b));
    }

    #[test]
    fn date_is_plausible() {
        let [year, month, day, secs] = date_now();
        assert!(year >= 2024.0 && (0.0..12.0).contains(&month) && (1.0..=31.0).contains(&day));
        assert!((0.0..86_400.0).contains(&secs));
    }
}
