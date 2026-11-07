// Native Metal Shading Language (MSL) shaders for Litty:
// - vs_main / fs_main: single-draw-call instanced cell renderer
// - vs_corner / fs_corner: rounded window corner cutout pass
// - vs_image / fs_image: inline images (Kitty graphics, Sixel)
// - vs_postfx: full-screen triangle vertex shader for custom post-processing passes

#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float2 screen;          // surface size in px
    float2 cell;            // cell size in px
    float2 atlas_size;      // atlas texture size in px
    float2 offset;          // padding in px
    uint cols;
    uint slots_per_row;
    uint2 cursor;
    uint cursor_visible;
    uint cursor_style;      // 0 = block, 1 = bar, 2 = underline, 3 = block_hollow
    uint default_fg;
    uint default_bg;
    uint tab_info;          // low 16 bits = tab_count (0 if single tab), high 16 bits = active_tab
    float bg_opacity;
    float underline_y;      // underline top, px from the cell top
    float strike_y;         // strikethrough center, px from the cell top
    uint cursor_color;      // OSC 12 color as 0xRRGGBB00 | 1, or 0 for the built-in color
    float unfocused_dim;    // 1 - unfocused-split-opacity
    uint unfocused_fill;    // 0xRRGGBB00 | 1, or 0 for the default background
    float min_contrast;     // minimum-contrast ratio, 1 = off
    uint progress;          // OSC 9;4 of the active pane: state << 8 | percent, 0 = none
    uint progress_row;      // active pane's first instance row,
    uint progress_col;      // first column
    uint progress_cols;     // and width
    uint bell_border;       // 1 = outline the window (bell-features = border)
    uint flip_y;            // 1 = draw upside down (into a custom-shader texture)
    uint layer;             // 0 = everything, 1 = backgrounds only, 2 = text only (images under the text)
    uint pane_count;
    uint tab_drag;          // dragged tab + 1 (drawn above the others), 0 = none
    uint tab_flags;         // bit 0 = a title bar band (window buttons, terminal-colored); bits 1..2 = bands at the top
    float corner_radius;    // rounded window corners in pixels (0 = square)
    uint tab_pad2;
    int4 tab_shifts[4];     // column shift of each of the first 16 tabs from its equal-width slot
};

static_assert(sizeof(Uniforms) == 208, "Uniforms must match 208-byte Rust repr(C) layout");

struct Viewport {
    uint col;
    uint row;
    uint cols;
    uint rows;
    uint extra_start;
    float shift;
    float top;
    float bottom;
    uint divider;
    uint pad;
};

static_assert(sizeof(Viewport) == 40, "Viewport must match 40-byte Rust repr(C) layout");

struct Instance {
    uint glyph;
    uint fg;
    uint bg;
};

static_assert(sizeof(Instance) == 12, "Instance must match 12-byte Rust repr(C) layout");

struct VsOut {
    float4 pos [[position]];
    float2 uv;
    float2 local;
    float3 fg [[flat]];
    float3 bg [[flat]];
    // bits 0..3 = underline style, bit 3 = strikethrough, bits 4..7 = dividers, bit 7 = tab_bar_row,
    // bits 8..10 = cursor style, bit 11 = fixed bottom divider, bit 12 = unfocused split,
    // bit 13 = color glyph, bit 14 = scrollbar overlay, bits 16..31 = sxy.y
    uint flags [[flat]];
    uint col [[flat]];
    float3 ul [[flat]];
    uint row [[flat]];
    // Unstretched top-left of the cell in px and its atlas slot origin in uv:
    // edge quads may be stretched over the padding (bell border).
    float2 origin [[flat]];
    float2 uv0 [[flat]];
    float2 clip_y [[flat]];
};

static inline int tab_shift(constant Uniforms& u, uint i) {
    return u.tab_shifts[i / 4u][i % 4u];
}

static inline float3 unpack(uint c) {
    return float3(float((c >> 24u) & 255u), float((c >> 16u) & 255u), float((c >> 8u) & 255u)) / 255.0f;
}

// Cursor color: set by OSC 12, else Ghostty's slate-lavender.
static inline float3 cursor_rgb(constant Uniforms& u) {
    return ((u.cursor_color & 1u) != 0u) ? unpack(u.cursor_color) : float3(0.44f, 0.47f, 0.59f);
}

// Exact sRGB <-> Linear color conversions for gamma-correct alpha blending
// (matches Ghostty `alpha-blending = native/linear`, eliminates dark halos).
static inline float3 srgb_to_linear(float3 c) {
    bool3 cutoff = c <= float3(0.04045f);
    float3 higher = pow((c + float3(0.055f)) / 1.055f, float3(2.4f));
    float3 lower = c / 12.92f;
    return select(higher, lower, cutoff);
}

static inline float3 linear_to_srgb(float3 c) {
    bool3 cutoff = c <= float3(0.0031308f);
    float3 higher = 1.055f * pow(max(c, float3(0.0f)), float3(1.0f / 2.4f)) - 0.055f;
    float3 lower = c * 12.92f;
    return select(higher, lower, cutoff);
}

// WCAG relative luminance and contrast ratio.
static inline float luminance(float3 c) {
    return dot(srgb_to_linear(c), float3(0.2126f, 0.7152f, 0.0722f));
}

static inline float contrast(float3 a, float3 b) {
    float la = luminance(a);
    float lb = luminance(b);
    return (max(la, lb) + 0.05f) / (min(la, lb) + 0.05f);
}

// Text color with at least `u.min_contrast` against `bg`: black or white,
// whichever contrasts more, when the color itself falls short.
static inline float3 ensure_contrast(constant Uniforms& u, float3 fg, float3 bg) {
    if (u.min_contrast <= 1.0f || contrast(fg, bg) >= u.min_contrast) {
        return fg;
    }
    float white = contrast(float3(1.0f), bg);
    float black = contrast(float3(0.0f), bg);
    return (white > black) ? float3(1.0f) : float3(0.0f);
}

// Coverage (0..1) of an underline of `style` (1 single, 2 double, 3 curly, 4 dotted,
// 5 dashed) at `px` inside the cell. Patterns repeat per cell so they tile seamlessly.
static inline float underline_coverage(constant Uniforms& u, uint style, float2 px, float t) {
    if (style == 0u) {
        return 0.0f;
    }
    float amp = max(1.0f, floor(t * 1.25f));
    float gap = max(2.0f, 2.0f * t);
    float room = t;
    if (style == 2u) {
        room = 2.0f * t + gap;
    } else if (style == 3u) {
        room = 2.0f * amp + t;
    }
    float y0 = max(0.0f, min(u.underline_y, u.cell.y - room));
    bool in_line = px.y >= y0 && px.y < y0 + t;
    if (style == 2u) {
        float y1 = y0 + t + gap;
        return (in_line || (px.y >= y1 && px.y < y1 + t)) ? 1.0f : 0.0f;
    }
    if (style == 3u) {
        float k = 6.2831853f / u.cell.x;
        float center = y0 + amp + t * 0.5f;
        float wave = center - amp * sin(px.x * k);
        float slope = amp * k * cos(px.x * k);
        float d = abs(px.y - wave) / sqrt(1.0f + slope * slope);
        return clamp(t * 0.5f + 0.5f - d, 0.0f, 1.0f);
    }
    if (!in_line) {
        return 0.0f;
    }
    if (style == 4u) {
        return (fract(floor(px.x / t) * 0.5f) < 0.25f) ? 1.0f : 0.0f;
    }
    if (style == 5u) {
        float x = px.x / u.cell.x;
        return (x >= 0.2f && x < 0.8f) ? 1.0f : 0.0f;
    }
    return 1.0f;
}

vertex VsOut vs_main(
    uint vi [[vertex_id]],
    uint ii [[instance_id]],
    device const Instance* instances [[buffer(0)]],
    constant Uniforms& u [[buffer(1)]],
    device const Viewport* viewports [[buffer(2)]]
) {
    Instance inst = instances[ii];
    uint glyph = inst.glyph;
    uint fg = inst.fg;
    uint bg = inst.bg;

    float2 corner = float2(float(vi & 1u), float(vi >> 1u));
    // Extra instances follow the terminal cells and draw a capsule in pixel space.
    if ((bg & 255u) == 48u) {
        float2 origin = float2(as_type<float>(glyph), as_type<float>(fg));
        float2 size = float2(10.0f, float(bg >> 8u));
        float2 px = origin + corner * size;
        VsOut bar = {};
        bar.pos = float4(px.x / u.screen.x * 2.0f - 1.0f, 1.0f - px.y / u.screen.y * 2.0f, 0.0f, 1.0f);
        if (u.flip_y != 0u) { bar.pos.y = -bar.pos.y; }
        bar.flags = 16384u;
        bar.origin = origin;
        bar.uv0 = size;
        return bar;
    }
    uint cols = max(u.cols, 1u);
    uint col = ii % cols;
    uint row = ii / cols;
    float shift = 0.0f;
    float2 clip_y = float2(0.0f, u.screen.y);
    uint divider = 0u;
    bool is_preview = (bg & 16u) != 0u;
    if (!is_preview) {
        for (uint i = 0u; i < u.pane_count; i++) {
            Viewport pane = viewports[i];
            bool extra = ii >= pane.extra_start && ii < pane.extra_start + pane.cols;
            if (extra) {
                if (pane.shift == 0.0f) {
                    VsOut hidden = {};
                    hidden.pos = float4(2.0f, 2.0f, 0.0f, 1.0f);
                    return hidden;
                }
                col = pane.col + ii - pane.extra_start;
                row = pane.row + pane.rows;
            }
            if (extra || (col >= pane.col && col < pane.col + pane.cols && row >= pane.row && row < pane.row + pane.rows)) {
                shift = pane.shift;
                clip_y = float2(pane.top, pane.bottom);
                if (shift != 0.0f && (pane.divider & 1u) != 0u) {
                    divider = 2048u | (((pane.divider & 2u) != 0u) ? 64u : 0u);
                }
                break;
            }
        }
    }
    uint tab_count = u.tab_info & 0xFFFFu;
    // Bands at the top, two cell rows each: the title bar and/or the tab strip.
    uint bands = (u.tab_flags >> 1u) & 3u;
    if (bands == 0u && tab_count > 0u) {
        bands = 1u;
    }
    bool is_tab_row = row < bands;

    float2 px;
    if (is_tab_row) {
        float x = (float(col) + corner.x) * u.cell.x + u.offset.x;
        if (col == 0u && corner.x == 0.0f) {
            x = 0.0f;
        } else if (u.cols > 0u && col == u.cols - 1u && corner.x == 1.0f) {
            x = u.screen.x;
        }
        float y0 = (row > 0u) ? (u.cell.y * 2.0f * float(row) + u.offset.y) : 0.0f;
        float y1 = u.cell.y * 2.0f * float(row + 1u) + u.offset.y;
        px = float2(x, mix(y0, y1, corner.y));
    } else if (bands > 0u) {
        px = float2(
            (float(col) + corner.x) * u.cell.x + u.offset.x,
            (float(row + bands) + corner.y) * u.cell.y + u.offset.y
        );
    } else {
        px = (float2(float(col), float(row)) + corner) * u.cell + u.offset;
    }
    px.y += shift;
    float2 origin = px - corner * u.cell;
    // Bell border: stretch the outermost cells over the padding so it can be drawn there.
    if (u.bell_border != 0u && !is_tab_row && !is_preview) {
        if (col == 0u && corner.x == 0.0f) {
            px.x = 0.0f;
        } else if (corner.x == 1.0f && px.x + u.cell.x > u.screen.x) {
            px.x = u.screen.x;
        }
        if (row == 0u && corner.y == 0.0f) {
            px.y = 0.0f;
        } else if (corner.y == 1.0f && px.y + u.cell.y > u.screen.y) {
            px.y = u.screen.y;
        }
    }

    VsOut out = {};
    out.pos = float4(px.x / u.screen.x * 2.0f - 1.0f, 1.0f - px.y / u.screen.y * 2.0f, 0.0f, 1.0f);
    if (u.flip_y != 0u) {
        out.pos.y = -out.pos.y;
    }
    uint slot = glyph & 0xFFFFu;
    float2 sxy = float2(float(slot % u.slots_per_row), float(slot / u.slots_per_row));
    out.uv = (sxy + corner) * u.cell / u.atlas_size;
    out.local = corner;
    out.origin = origin;
    out.uv0 = sxy * u.cell / u.atlas_size;
    out.clip_y = clip_y;

    float3 f = ((fg & 0x80u) == 0u) ? unpack(u.default_fg) : unpack(fg);
    float3 b = ((bg & 0x80u) == 0u) ? unpack(u.default_bg) : unpack(bg);
    bool is_cursor = !is_tab_row && !is_preview && u.cursor_visible != 0u && col == u.cursor.x && row == u.cursor.y;
    if (is_cursor && u.cursor_style == 0u) {
        f = unpack(u.default_bg);
        b = cursor_rgb(u);
    }
    if (!is_tab_row && !is_preview) {
        f = ensure_contrast(u, f, b);
    }
    out.fg = f;
    out.bg = b;
    uint ul = (glyph >> 19u) & 0xFFFu;
    out.ul = ((glyph >> 31u) != 0u)
        ? (float3(float(ul >> 8u), float((ul >> 4u) & 15u), float(ul & 15u)) / 15.0f)
        : f;
    uint cursor_flag = is_cursor ? ((u.cursor_style + 1u) << 8u) : 0u;
    uint divider_flags = (glyph >> 16u) & 7u;
    out.flags = (bg & 15u)
        | (((bg & 64u) != 0u) ? 4096u : 0u)
        | (((fg & 4u) != 0u) ? 8192u : 0u)
        | (is_tab_row ? 128u : 0u)
        | (divider_flags << 4u)
        | cursor_flag
        | divider
        | ((slot / u.slots_per_row) << 16u);
    out.col = col;
    out.row = row;
    return out;
}

struct CornerOut {
    float4 pos [[position]];
};

vertex CornerOut vs_corner(uint i [[vertex_id]]) {
    float2 p = float2(float(i & 1u), float(i >> 1u));
    CornerOut out;
    out.pos = float4(p.x * 2.0f - 1.0f, 1.0f - p.y * 2.0f, 0.0f, 1.0f);
    return out;
}

fragment float4 fs_corner(
    CornerOut v [[stage_in]],
    constant Uniforms& u [[buffer(1)]]
) {
    float r = u.corner_radius;
    float2 half_size = u.screen * 0.5f;
    float2 q = abs(v.pos.xy - half_size) - (half_size - float2(r));
    if (q.x <= 0.0f || q.y <= 0.0f) {
        discard_fragment();
    }
    float cut = clamp(length(q) - r + 0.5f, 0.0f, 1.0f);
    if (cut <= 0.0f) {
        discard_fragment();
    }
    return float4(0.0f, 0.0f, 0.0f, cut);
}

fragment float4 fs_main(
    VsOut v [[stage_in]],
    constant Uniforms& u [[buffer(1)]],
    texture2d<float> atlas [[texture(0)]],
    texture2d<float> color_atlas [[texture(1)]],
    sampler samp [[sampler(0)]]
) {
    float2 fp = float2(v.pos.x, (u.flip_y != 0u) ? (u.screen.y - v.pos.y) : v.pos.y);
    if ((v.flags & 16384u) != 0u) {
        if (u.layer == 1u) { discard_fragment(); }
        float2 half_size = v.uv0 * 0.5f;
        float radius = min(half_size.x, half_size.y);
        float2 q = abs(fp - v.origin - half_size) - (half_size - float2(radius));
        float distance = length(max(q, float2(0.0f))) + min(max(q.x, q.y), 0.0f) - radius;
        return float4(0.62f, 0.64f, 0.70f, 0.75f * clamp(0.5f - distance, 0.0f, 1.0f));
    }
    if (fp.y < v.clip_y.x || fp.y >= v.clip_y.y) { discard_fragment(); }
    // Unseen bell: an amber outline around the window.
    if (u.bell_border != 0u) {
        float w = max(2.0f, floor(u.cell.y / 8.0f));
        float2 p = fp;
        if (p.x < w || p.y < w || p.x >= u.screen.x - w || p.y >= u.screen.y - w) {
            return float4(0.96f, 0.69f, 0.25f, 1.0f);
        }
    }

    if ((v.flags & 128u) != 0u) {
        if (u.layer == 2u) {
            discard_fragment();
        }
        uint tab_count = u.tab_info & 0xFFFFu;
        uint active_tab = u.tab_info >> 16u;
        float bar_h = floor(u.cell.y * 1.56f);
        float3 term_bg = unpack(u.default_bg);
        bool titlebar = (u.tab_flags & 1u) != 0u;
        float strip_top = u.cell.y * 2.0f + u.offset.y;
        bool in_title = titlebar && fp.y < strip_top;
        float2 band_fp = float2(fp.x, (titlebar && !in_title) ? (fp.y - strip_top) : fp.y);

        if (band_fp.y >= bar_h) {
            return float4(term_bg, 1.0f);
        }

        float3 bar_bg = titlebar ? term_bg : float3(0.110f, 0.114f, 0.125f);
        float3 col_rgb = bar_bg;
        if (band_fp.y >= bar_h - 1.0f && !titlebar) {
            return float4(float3(0.07f, 0.075f, 0.085f), 1.0f);
        }

        float cy = bar_h * 0.5f;
        float half_h = floor(u.cell.y * 0.58f);

        if (in_title) {
            if (u.cols >= 6u && v.col + 6u >= u.cols) {
                uint slot = (v.col + 6u - u.cols) / 2u;
                float bx = u.offset.x + (float(u.cols - 6u + slot * 2u) + 1.0f) * u.cell.x;
                float2 d = band_fp - float2(bx, cy);
                float arm = floor(u.cell.y * 0.30f);
                float thick = max(1.0f, rint(u.cell.y / 12.0f)) * 0.55f;
                float luma = dot(term_bg, float3(0.299f, 0.587f, 0.114f));
                float3 icon = (luma > 0.5f) ? float3(0.14f, 0.14f, 0.16f) : float3(0.86f, 0.87f, 0.90f);
                float cov = 0.0f;
                if (slot == 0u) {
                    if (abs(d.x) <= arm && abs(d.y - arm * 0.6f) <= thick) { cov = 1.0f; }
                } else if (slot == 1u) {
                    float m = max(abs(d.x), abs(d.y));
                    if (m <= arm && m >= arm - thick * 2.0f) { cov = 1.0f; }
                } else {
                    if (max(abs(d.x), abs(d.y)) <= arm) {
                        cov = clamp(1.0f - (abs(abs(d.x) - abs(d.y)) * 0.7071f - thick), 0.0f, 1.0f);
                    }
                }
                return float4(mix(bar_bg, icon, cov), 1.0f);
            }
            float text_y0 = floor(cy - u.cell.y * 0.5f);
            float gy = band_fp.y - text_y0;
            if (gy >= 0.0f && gy < u.cell.y) {
                float luma = dot(term_bg, float3(0.299f, 0.587f, 0.114f));
                float3 title_fg = (luma > 0.5f) ? float3(0.12f, 0.12f, 0.14f) : v.fg;
                float slot_y = float(v.flags >> 16u);
                float2 uv = float2(v.uv.x, (slot_y * u.cell.y + gy) / u.atlas_size.y);
                float ga = atlas.sample(samp, uv, level(0.0f)).r;
                if (ga >= 1.0f) {
                    col_rgb = title_fg;
                } else if (ga > 0.0f) {
                    col_rgb = linear_to_srgb(mix(srgb_to_linear(col_rgb), srgb_to_linear(title_fg), ga));
                }
            }
            return float4(col_rgb, 1.0f);
        }

        uint tab_cols_total = (u.cols > 2u) ? (u.cols - 2u) : 1u;
        float plus_cx = u.offset.x + (float(tab_cols_total) + 1.0f) * u.cell.x;

        float d_plus = length(band_fp - float2(plus_cx, cy)) - half_h;
        if (d_plus < 1.0f) {
            float fill_a = clamp(0.5f - d_plus, 0.0f, 1.0f);
            float border_a = clamp(1.0f - abs(d_plus + 0.6f), 0.0f, 1.0f);
            float3 btn_fill = float3(0.165f, 0.169f, 0.188f);
            float3 btn_border = float3(0.290f, 0.298f, 0.329f);
            float3 btn_col = mix(btn_fill, btn_border, border_a);

            float2 dp = abs(band_fp - float2(plus_cx, cy));
            float arm = floor(u.cell.y * 0.22f);
            float thick = max(1.0f, rint(u.cell.y / 16.0f)) * 0.55f;
            if ((dp.x <= arm && dp.y <= thick) || (dp.y <= arm && dp.x <= thick)) {
                btn_col = float3(0.92f, 0.92f, 0.95f);
            }
            col_rgb = mix(bar_bg, btn_col, fill_a);
            return float4(col_rgb, 1.0f);
        }

        uint owner = 0u;
        bool owned = false;
        int shift = 0;
        if (tab_count > 16u) {
            owner = (v.col * tab_count) / tab_cols_total;
            uint next_start = ((owner + 1u) * tab_cols_total) / tab_count;
            if (v.col >= next_start && owner + 1u < tab_count) {
                owner = owner + 1u;
            }
            owned = true;
        } else {
            for (uint i = 0u; i < tab_count + 1u; i = i + 1u) {
                uint t = i;
                if (i == tab_count) {
                    if (u.tab_drag == 0u) { break; }
                    t = u.tab_drag - 1u;
                } else if (i + 1u == u.tab_drag) {
                    continue;
                }
                int s = tab_shift(u, t);
                int a = int((t * tab_cols_total) / tab_count) + s;
                int b = int(((t + 1u) * tab_cols_total) / tab_count) + s;
                if (a >= 0 && b <= int(tab_cols_total) && int(v.col) >= a && int(v.col) < b) {
                    owner = t;
                    owned = true;
                    shift = s;
                }
            }
        }
        if (v.col < tab_cols_total && owned) {
            uint tab_i = owner;
            int c0 = int((tab_i * tab_cols_total) / tab_count) + shift;
            int c1 = int(((tab_i + 1u) * tab_cols_total) / tab_count) + shift;

            float x0 = u.offset.x + float(c0) * u.cell.x + 3.0f;
            float x1 = u.offset.x + float(c1) * u.cell.x - 3.0f;
            float pill_cx = (x0 + x1) * 0.5f;
            float half_w = max((x1 - x0) * 0.5f, half_h);

            float2 q = abs(band_fp - float2(pill_cx, cy)) - float2(max(half_w - half_h, 0.0f), 0.0f);
            float d_pill = length(max(q, float2(0.0f))) + min(max(q.x, q.y), 0.0f) - half_h;

            bool is_active = tab_i == active_tab;
            bool is_dragged = u.tab_drag == tab_i + 1u;
            float3 pill_fill = is_active
                ? float3(0.204f, 0.208f, 0.231f)
                : float3(0.141f, 0.145f, 0.161f);
            float3 pill_border = is_active
                ? float3(0.365f, 0.373f, 0.412f)
                : float3(0.208f, 0.212f, 0.235f);
            if (is_dragged) {
                pill_fill = float3(0.255f, 0.259f, 0.290f);
                pill_border = float3(0.482f, 0.490f, 0.541f);
            }

            float fill_a = clamp(0.5f - d_pill, 0.0f, 1.0f);
            float border_a = clamp(1.0f - abs(d_pill + 0.65f), 0.0f, 1.0f);
            float3 pill_col = mix(pill_fill, pill_border, border_a);

            float text_y0 = floor(cy - u.cell.y * 0.5f);
            float gy = band_fp.y - text_y0;
            if (gy >= 0.0f && gy < u.cell.y && d_pill < -1.5f) {
                float slot_y = float(v.flags >> 16u);
                float2 uv = float2(v.uv.x, (slot_y * u.cell.y + gy) / u.atlas_size.y);
                float ga = atlas.sample(samp, uv, level(0.0f)).r;
                if (ga >= 1.0f) {
                    pill_col = v.fg;
                } else if (ga > 0.0f) {
                    float3 pill_lin = srgb_to_linear(pill_col);
                    float3 fg_lin = srgb_to_linear(v.fg);
                    pill_col = linear_to_srgb(mix(pill_lin, fg_lin, ga));
                }
            }

            col_rgb = mix(bar_bg, pill_col, fill_a);
        }
        return float4(col_rgb, 1.0f);
    }

    float2 local = (fp - v.origin) / u.cell;
    if (any(local < float2(0.0f)) || any(local >= float2(1.0f))) {
        if (u.layer != 0u) {
            discard_fragment();
        }
        return float4(unpack(u.default_bg), u.bg_opacity);
    }
    float2 glyph_uv = v.uv0 + local * u.cell / u.atlas_size;
    float a = atlas.sample(samp, glyph_uv, level(0.0f)).r;
    bool is_color = (v.flags & 8192u) != 0u;
    float4 emoji = float4(0.0f);
    if (is_color) {
        emoji = color_atlas.sample(samp, glyph_uv, level(0.0f));
        a = emoji.a;
    }
    float2 px = local * u.cell;
    float thickness = max(1.0f, floor(u.cell.y / 14.0f));
    if ((v.flags & 8u) != 0u && abs(px.y - u.strike_y) <= thickness * 0.5f) {
        a = 1.0f;
    }
    if (u.layer == 1u) {
        a = 0.0f;
    }
    float3 base_bg = (u.layer == 2u) ? v.fg : v.bg;
    bool solid = false;
    float3 cell_color;
    if (a <= 0.0f) {
        cell_color = base_bg;
    } else if (a >= 1.0f) {
        cell_color = v.fg;
    } else {
        float3 fg_lin = srgb_to_linear(v.fg);
        float3 bg_lin = srgb_to_linear(base_bg);
        float3 cell_lin = mix(bg_lin, fg_lin, a);
        cell_color = linear_to_srgb(cell_lin);
    }
    if (is_color && a > 0.0f) {
        cell_color = (u.layer == 2u) ? (emoji.rgb / a) : (emoji.rgb + base_bg * (1.0f - a));
    }

    float ul_cov = 0.0f;
    if (u.layer != 1u) {
        ul_cov = underline_coverage(u, v.flags & 7u, px, thickness);
    }
    if (ul_cov > 0.0f) {
        cell_color = linear_to_srgb(mix(srgb_to_linear(cell_color), srgb_to_linear(v.ul), ul_cov));
        a = max(a, ul_cov);
    }

    uint c_style = (v.flags >> 8u) & 7u;
    if (c_style != 0u) {
        float3 cursor_color = cursor_rgb(u);
        if (c_style == 2u) {
            if (local.x * u.cell.x <= 2.0f) {
                cell_color = cursor_color;
                solid = true;
            }
        } else if (c_style == 3u) {
            if (local.y * u.cell.y >= u.cell.y - 2.0f) {
                cell_color = cursor_color;
                solid = true;
            }
        } else if (c_style == 4u) {
            float lx = local.x * u.cell.x;
            float ly = local.y * u.cell.y;
            if (lx <= 1.5f || lx >= u.cell.x - 1.5f || ly <= 1.5f || ly >= u.cell.y - 1.5f) {
                cell_color = cursor_color;
                solid = true;
            }
        }
    }

    bool on_divider = false;
    bool is_div_r = (v.flags & 16u) != 0u;
    bool is_div_b = (v.flags & 32u) != 0u;
    bool fixed_bottom = (v.flags & 2048u) != 0u;
    bool is_div_act = (v.flags & 64u) != 0u;
    if (is_div_r || is_div_b || fixed_bottom) {
        float3 div_color = is_div_act
            ? float3(0.420f, 0.451f, 0.549f)
            : float3(0.208f, 0.212f, 0.251f);
        float rx = local.x * u.cell.x;
        float ry = local.y * u.cell.y;
        if (is_div_r && rx >= u.cell.x - 1.0f) {
            cell_color = div_color;
            on_divider = true;
        }
        if ((fixed_bottom && fp.y >= v.clip_y.y - 1.0f) || (is_div_b && ry >= u.cell.y - 1.0f)) {
            cell_color = div_color;
            on_divider = true;
        }
    }

    uint p_state = u.progress >> 8u;
    if (p_state != 0u && (v.flags & 128u) == 0u && v.row == u.progress_row
        && u.progress_cols > 0u
        && v.col >= u.progress_col && v.col < u.progress_col + u.progress_cols
        && local.y * u.cell.y < max(2.0f, floor(u.cell.y / 8.0f))) {
        float x = (float(v.col - u.progress_col) + local.x) / float(max(u.progress_cols, 1u));
        bool filled = p_state == 3u || x * 100.0f < float(u.progress & 255u);
        float3 bar = float3(0.36f, 0.56f, 0.98f);
        if (p_state == 2u) {
            bar = float3(0.93f, 0.33f, 0.36f);
        } else if (p_state == 4u) {
            bar = float3(0.95f, 0.76f, 0.30f);
        } else if (p_state == 3u) {
            bar = float3(0.30f, 0.42f, 0.68f);
        }
        if (filled) {
            cell_color = bar;
            solid = true;
        }
    }

    if ((v.flags & 4096u) != 0u && u.unfocused_dim > 0.0f) {
        float3 fill = ((u.unfocused_fill & 1u) != 0u) ? unpack(u.unfocused_fill) : unpack(u.default_bg);
        cell_color = mix(cell_color, fill, u.unfocused_dim);
    }

    float out_alpha = 1.0f;
    float3 def_bg = unpack(u.default_bg);
    bool default_cell = !on_divider && (v.flags & 128u) == 0u && c_style != 1u
        && abs(v.bg.r - def_bg.r) < 0.002f && abs(v.bg.g - def_bg.g) < 0.002f && abs(v.bg.b - def_bg.b) < 0.002f;
    if (u.layer == 2u) {
        out_alpha = (solid || on_divider) ? 1.0f : a;
        if (out_alpha <= 0.0f) {
            discard_fragment();
        }
    } else if (default_cell) {
        if (u.layer == 1u && !solid) {
            discard_fragment();
        }
        out_alpha = mix(u.bg_opacity, 1.0f, a);
    }

    return float4(cell_color, out_alpha);
}

// ============================================================================
// Inline Images (Kitty graphics, Sixel)
// ============================================================================

struct ImageUniforms {
    float2 screen;
    uint flip_y;
    uint pad;
};

struct ImageQuad {
    float4 rect;
    float4 uv;
};

struct ImageVsOut {
    float4 pos [[position]];
    float2 uv;
};

vertex ImageVsOut vs_image(
    uint vi [[vertex_id]],
    uint ii [[instance_id]],
    device const ImageQuad* quads [[buffer(0)]],
    constant ImageUniforms& u [[buffer(1)]]
) {
    ImageQuad q = quads[ii];
    float2 corner = float2(float(vi & 1u), float(vi >> 1u));
    float2 px = q.rect.xy + corner * q.rect.zw;
    ImageVsOut out;
    out.pos = float4(px.x / u.screen.x * 2.0f - 1.0f, 1.0f - px.y / u.screen.y * 2.0f, 0.0f, 1.0f);
    if (u.flip_y != 0u) {
        out.pos.y = -out.pos.y;
    }
    out.uv = mix(q.uv.xy, q.uv.zw, corner);
    return out;
}

fragment float4 fs_image(
    ImageVsOut v [[stage_in]],
    texture2d<float> tex [[texture(0)]],
    sampler samp [[sampler(0)]]
) {
    return tex.sample(samp, v.uv);
}
