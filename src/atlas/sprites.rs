//! Procedural glyphs drawn to exact cell size with 4×4 supersampling, like
//! Ghostty's `font/sprite`: Powerline (U+E0A0..=U+E0D7), Box Drawing
//! (U+2500..=U+257F), Block Elements (U+2580..=U+259F), geometric triangles
//! (U+25E2..=U+25E5), Braille (U+2800..=U+28FF), and a few Nerd Font icons.


/// Codepoints drawn procedurally by [`draw_builtin_sprite`].
pub(super) fn is_builtin_sprite(cp: u32) -> bool {
    matches!(cp, 0xE0A0..=0xE0D7 | 0x2500..=0x259F | 0x25E2..=0x25E5 | 0x2800..=0x28FF)
}

pub(super) fn draw_builtin_sprite(buf: &mut [u8], cw: u32, ch: u32, cp: u32) -> bool {
    if cw == 0 || ch == 0 {
        return false;
    }
    match cp {
        0xE0A0..=0xE0D7 => draw_powerline(buf, cw, ch, cp),
        0x2580..=0x259F | 0x25E2..=0x25E5 => draw_block_element(buf, cw, ch, cp),
        0x2500..=0x257F => draw_box_drawing(buf, cw, ch, cp),
        0x2800..=0x28FF => {
            draw_braille(buf, cw, ch, (cp - 0x2800) as u8);
            true
        }
        _ => false,
    }
}

/// Evaluate a 4×4 supersampled coverage predicate `(x, y)` in pixel coordinates
/// where `x in [0.0, cw]` and `y in [0.0, ch]`.
fn rasterize_supersampled(buf: &mut [u8], cw: u32, ch: u32, mut f: impl FnMut(f32, f32) -> bool) {
    for py in 0..ch {
        for px in 0..cw {
            let mut hits = 0u32;
            for sy in 0..4 {
                let y = py as f32 + (sy as f32 + 0.5) * 0.25;
                for sx in 0..4 {
                    let x = px as f32 + (sx as f32 + 0.5) * 0.25;
                    if f(x, y) {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let a = ((hits * 255 + 8) / 16) as u8;
                let dst = &mut buf[(py * cw + px) as usize];
                *dst = (*dst).max(a);
            }
        }
    }
}

#[inline]
fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = bx - ax;
    let dy = by - ay;
    let len_sq = dx * dx + dy * dy;
    let t = if len_sq > 1e-6 {
        ((px - ax) * dx + (py - ay) * dy) / len_sq
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    let cx = ax + t * dx;
    let cy = ay + t * dy;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

fn draw_powerline(buf: &mut [u8], cw: u32, ch: u32, cp: u32) -> bool {
    let w = cw as f32;
    let h = ch as f32;
    let mid_y = h * 0.5;
    let stroke = (w * 0.12).max(1.4);
    let half_s = stroke * 0.5;

    match cp {
        //  U+E0A0: Powerline Git Branch symbol
        0xE0A0 => {
            let x1 = w * 0.28;
            let x2 = w * 0.72;
            let y_top = h * 0.24;
            let y_bot = h * 0.76;
            let y_rt = h * 0.35;
            let y_join = h * 0.60;
            let r_outer = (w * 0.155).max(1.6);
            let r_inner = if w >= 12.0 { r_outer * 0.36 } else { 0.0 };
            let stem_r = (w * 0.082).max(0.85);

            rasterize_supersampled(buf, cw, ch, |x, y| {
                let d_tl = ((x - x1).powi(2) + (y - y_top).powi(2)).sqrt();
                let d_bl = ((x - x1).powi(2) + (y - y_bot).powi(2)).sqrt();
                let d_tr = ((x - x2).powi(2) + (y - y_rt).powi(2)).sqrt();

                // Carve center holes of the 3 branch nodes first.
                if d_tl < r_inner || d_bl < r_inner || d_tr < r_inner {
                    return false;
                }
                if d_tl <= r_outer || d_bl <= r_outer || d_tr <= r_outer {
                    return true;
                }
                // Left vertical trunk between top-left and bottom-left nodes.
                if dist_to_segment(x, y, x1, y_top, x1, y_bot) <= stem_r {
                    return true;
                }
                // Right vertical drop from top-right node down to curve start.
                let y_curve_top = h * 0.45;
                if dist_to_segment(x, y, x2, y_rt, x2, y_curve_top) <= stem_r {
                    return true;
                }
                // Smooth S-curve connecting (x2, y_curve_top) to (x1, y_join).
                let steps = 8;
                let mut prev_x = x2;
                let mut prev_y = y_curve_top;
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let u = 1.0 - t;
                    // Cubic Bezier with vertical tangent at x2 and smooth join at x1.
                    let bx = u * u * u * x2
                        + 3.0 * u * u * t * x2
                        + 3.0 * u * t * t * (x1 + (x2 - x1) * 0.25)
                        + t * t * t * x1;
                    let by = u * u * u * y_curve_top
                        + 3.0 * u * u * t * (y_join - (y_join - y_curve_top) * 0.15)
                        + 3.0 * u * t * t * y_join
                        + t * t * t * y_join;
                    if dist_to_segment(x, y, prev_x, prev_y, bx, by) <= stem_r {
                        return true;
                    }
                    prev_x = bx;
                    prev_y = by;
                }
                false
            });
            true
        }

        //  U+E0A1: LN (line number) symbol
        0xE0A1 => {
            let sw = (w * 0.075).max(0.75);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                // 'L' in upper-left: (0.18w, 0.18h) -> (0.18w, 0.52h) -> (0.52w, 0.52h)
                let l1 = dist_to_segment(x, y, w * 0.18, h * 0.18, w * 0.18, h * 0.52) <= sw;
                let l2 = dist_to_segment(x, y, w * 0.18, h * 0.52, w * 0.52, h * 0.52) <= sw;
                // 'N' in lower-right: (0.42w, 0.82h) -> (0.42w, 0.46h) -> (0.82w, 0.82h) -> (0.82w, 0.46h)
                let n1 = dist_to_segment(x, y, w * 0.42, h * 0.82, w * 0.42, h * 0.46) <= sw;
                let n2 = dist_to_segment(x, y, w * 0.42, h * 0.46, w * 0.82, h * 0.82) <= sw;
                let n3 = dist_to_segment(x, y, w * 0.82, h * 0.82, w * 0.82, h * 0.46) <= sw;
                l1 || l2 || n1 || n2 || n3
            });
            true
        }

        //  U+E0A2: Padlock (read-only) symbol
        0xE0A2 => {
            let cx = w * 0.5;
            let body_l = w * 0.18;
            let body_r = w * 0.82;
            let body_t = h * 0.46;
            let body_b = h * 0.82;
            let arch_cy = body_t;
            let arch_r = w * 0.22;
            let arch_sw = (w * 0.075).max(0.8);
            let hole_r = (w * 0.07).max(0.75);
            let hole_cy = h * 0.60;

            rasterize_supersampled(buf, cw, ch, |x, y| {
                // Body with slightly rounded corners
                if x >= body_l && x <= body_r && y >= body_t && y <= body_b {
                    // Keyhole cutout
                    let d_hole = ((x - cx).powi(2) + (y - hole_cy).powi(2)).sqrt();
                    if d_hole <= hole_r {
                        return false;
                    }
                    if dist_to_segment(x, y, cx, hole_cy, cx, h * 0.73) <= hole_r * 0.65 {
                        return false;
                    }
                    return true;
                }
                // Shackle arch above body
                if y <= arch_cy {
                    let y_arch_top = h * 0.30;
                    if y >= y_arch_top {
                        let d_l = (x - (cx - arch_r)).abs();
                        let d_r = (x - (cx + arch_r)).abs();
                        if d_l <= arch_sw || d_r <= arch_sw {
                            return true;
                        }
                    } else {
                        let dy = (y - y_arch_top) * (arch_r / (h * 0.12).max(1.0));
                        let d = ((x - cx).powi(2) + dy * dy).sqrt();
                        if (d - arch_r).abs() <= arch_sw {
                            return true;
                        }
                    }
                }
                false
            });
            true
        }

        //  U+E0A3: CN (column number) symbol
        0xE0A3 => {
            let sw = (w * 0.075).max(0.75);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                // 'C' in upper-left
                let c_top = dist_to_segment(x, y, w * 0.48, h * 0.20, w * 0.18, h * 0.20) <= sw;
                let c_left = dist_to_segment(x, y, w * 0.18, h * 0.20, w * 0.18, h * 0.50) <= sw;
                let c_bot = dist_to_segment(x, y, w * 0.18, h * 0.50, w * 0.48, h * 0.50) <= sw;
                // 'N' in lower-right
                let n1 = dist_to_segment(x, y, w * 0.42, h * 0.82, w * 0.42, h * 0.46) <= sw;
                let n2 = dist_to_segment(x, y, w * 0.42, h * 0.46, w * 0.82, h * 0.82) <= sw;
                let n3 = dist_to_segment(x, y, w * 0.82, h * 0.82, w * 0.82, h * 0.46) <= sw;
                c_top || c_left || c_bot || n1 || n2 || n3
            });
            true
        }

        //  U+E0B0: Rightwards black arrowhead (solid triangle filling cell)
        0xE0B0 => {
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let dy = (y - mid_y).abs() / mid_y.max(0.5);
                x <= w * (1.0 - dy)
            });
            true
        }

        //  U+E0B1: Rightwards thin chevron
        0xE0B1 => {
            let tip_x = (w - half_s).max(0.0);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, 0.0, 0.0, tip_x, mid_y) <= half_s
                    || dist_to_segment(x, y, tip_x, mid_y, 0.0, h) <= half_s
            });
            true
        }

        //  U+E0B2: Leftwards black arrowhead (solid triangle filling cell)
        0xE0B2 => {
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let dy = (y - mid_y).abs() / mid_y.max(0.5);
                (w - x) <= w * (1.0 - dy)
            });
            true
        }

        //  U+E0B3: Leftwards thin chevron
        0xE0B3 => {
            let tip_x = half_s.min(w);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, w, 0.0, tip_x, mid_y) <= half_s
                    || dist_to_segment(x, y, tip_x, mid_y, w, h) <= half_s
            });
            true
        }

        //  U+E0B4: Right half-circle (rounded pill cap)
        0xE0B4 => {
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let nx = x / w.max(1.0);
                let ny = (y - mid_y) / mid_y.max(0.5);
                nx * nx + ny * ny <= 1.0
            });
            true
        }

        //  U+E0B5: Right half-circle thin arc
        0xE0B5 => {
            let rx = (w - half_s).max(1.0);
            let ry = (mid_y - half_s).max(1.0);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let nx = x / rx;
                let ny = (y - mid_y) / ry;
                let r = (nx * nx + ny * ny).sqrt();
                let approx_px = (r - 1.0).abs() * rx.min(ry);
                approx_px <= half_s
            });
            true
        }

        //  U+E0B6: Left half-circle (rounded pill cap)
        0xE0B6 => {
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let nx = (w - x) / w.max(1.0);
                let ny = (y - mid_y) / mid_y.max(0.5);
                nx * nx + ny * ny <= 1.0
            });
            true
        }

        //  U+E0B7: Left half-circle thin arc
        0xE0B7 => {
            let rx = (w - half_s).max(1.0);
            let ry = (mid_y - half_s).max(1.0);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let nx = (w - x) / rx;
                let ny = (y - mid_y) / ry;
                let r = (nx * nx + ny * ny).sqrt();
                let approx_px = (r - 1.0).abs() * rx.min(ry);
                approx_px <= half_s
            });
            true
        }

        //  U+E0B8: Lower-left triangle
        0xE0B8 => {
            rasterize_supersampled(buf, cw, ch, |x, y| (x / w) <= (y / h));
            true
        }

        //  U+E0B9 /  U+E0BF: Backslash thin diagonal
        0xE0B9 | 0xE0BF => {
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, 0.0, 0.0, w, h) <= half_s
            });
            true
        }

        //  U+E0BA: Lower-right triangle
        0xE0BA => {
            rasterize_supersampled(buf, cw, ch, |x, y| ((w - x) / w) <= (y / h));
            true
        }

        //  U+E0BB /  U+E0BD: Forwardslash thin diagonal
        0xE0BB | 0xE0BD => {
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, w, 0.0, 0.0, h) <= half_s
            });
            true
        }

        //  U+E0BC: Upper-left triangle
        0xE0BC => {
            rasterize_supersampled(buf, cw, ch, |x, y| (x / w) <= ((h - y) / h));
            true
        }

        //  U+E0BE: Upper-right triangle
        0xE0BE => {
            rasterize_supersampled(buf, cw, ch, |x, y| ((w - x) / w) <= ((h - y) / h));
            true
        }

        // Powerline Extra right-pointing variants (U+E0C0, E0C4, E0C6, E0C8, E0CC, E0CE, E0D2)
        0xE0C0 | 0xE0C4 | 0xE0C6 | 0xE0C8 | 0xE0CC | 0xE0CE | 0xE0D2 => {
            draw_powerline(buf, cw, ch, 0xE0B0)
        }
        // Powerline Extra left-pointing variants (U+E0C2, E0C5, E0C7, E0CA, E0D4)
        0xE0C2 | 0xE0C5 | 0xE0C7 | 0xE0CA | 0xE0D4 => draw_powerline(buf, cw, ch, 0xE0B2),
        // Powerline Extra thin variants
        0xE0C1 | 0xE0CD | 0xE0CF..=0xE0D1 => draw_powerline(buf, cw, ch, 0xE0B1),
        0xE0C3 | 0xE0C9 | 0xE0CB | 0xE0D3 | 0xE0D5..=0xE0D7 => draw_powerline(buf, cw, ch, 0xE0B3),

        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(buf: &mut [u8], cw: u32, ch: u32, x0: u32, y0: u32, x1: u32, y1: u32, val: u8) {
    let x0 = x0.min(cw);
    let x1 = x1.min(cw);
    let y0 = y0.min(ch);
    let y1 = y1.min(ch);
    for y in y0..y1 {
        for x in x0..x1 {
            let d = &mut buf[(y * cw + x) as usize];
            *d = (*d).max(val);
        }
    }
}

fn draw_block_element(buf: &mut [u8], cw: u32, ch: u32, cp: u32) -> bool {
    let w = cw as f32;
    let h = ch as f32;
    let hx = cw / 2;
    let hy = ch / 2;
    match cp {
        // ▀ Upper half block
        0x2580 => fill_rect(buf, cw, ch, 0, 0, cw, hy, 255),
        // ▁..=█ Lower 1/8 .. 8/8 blocks
        0x2581..=0x2588 => {
            let n = cp - 0x2580;
            let y0 = ((ch * (8 - n) + 4) / 8).min(ch);
            fill_rect(buf, cw, ch, 0, y0, cw, ch, 255);
        }
        // ▉..=▏ Left 7/8 .. 1/8 blocks
        0x2589..=0x258F => {
            let n = 8 - (cp - 0x2588);
            let x1 = ((cw * n + 4) / 8).max(1);
            fill_rect(buf, cw, ch, 0, 0, x1, ch, 255);
        }
        // ▐ Right half block
        0x2590 => fill_rect(buf, cw, ch, hx, 0, cw, ch, 255),
        // ░ Light shade (25%)
        0x2591 => fill_rect(buf, cw, ch, 0, 0, cw, ch, 64),
        // ▒ Medium shade (50%)
        0x2592 => fill_rect(buf, cw, ch, 0, 0, cw, ch, 128),
        // ▓ Dark shade (75%)
        0x2593 => fill_rect(buf, cw, ch, 0, 0, cw, ch, 192),
        // ▔ Upper 1/8 block
        0x2594 => fill_rect(buf, cw, ch, 0, 0, cw, (ch / 8).max(1), 255),
        // ▕ Right 1/8 block
        0x2595 => fill_rect(buf, cw, ch, cw - (cw / 8).max(1), 0, cw, ch, 255),
        // Quadrant blocks U+2596..=U+259F
        0x2596..=0x259F => {
            // Bitmask: bit0=UL, bit1=UR, bit2=LL, bit3=LR
            const QUADS: [u8; 10] = [
                0b0100, // 2596 ▖ LL
                0b1000, // 2597 ▗ LR
                0b0001, // 2598 ▘ UL
                0b1101, // 2599 ▙ UL + LL + LR
                0b1001, // 259A ▚ UL + LR
                0b0111, // 259B ▛ UL + UR + LL
                0b1011, // 259C ▜ UL + UR + LR
                0b0010, // 259D ▝ UR
                0b0110, // 259E ▞ UR + LL
                0b1110, // 259F ▟ UR + LL + LR
            ];
            let m = QUADS[(cp - 0x2596) as usize];
            if m & 1 != 0 {
                fill_rect(buf, cw, ch, 0, 0, hx, hy, 255);
            }
            if m & 2 != 0 {
                fill_rect(buf, cw, ch, hx, 0, cw, hy, 255);
            }
            if m & 4 != 0 {
                fill_rect(buf, cw, ch, 0, hy, hx, ch, 255);
            }
            if m & 8 != 0 {
                fill_rect(buf, cw, ch, hx, hy, cw, ch, 255);
            }
        }
        // ◢ U+25E2: Lower-right triangle
        0x25E2 => rasterize_supersampled(buf, cw, ch, |x, y| ((w - x) / w) <= (y / h)),
        // ◣ U+25E3: Lower-left triangle
        0x25E3 => rasterize_supersampled(buf, cw, ch, |x, y| (x / w) <= (y / h)),
        // ◤ U+25E4: Upper-left triangle
        0x25E4 => rasterize_supersampled(buf, cw, ch, |x, y| (x / w) <= ((h - y) / h)),
        // ◥ U+25E5: Upper-right triangle
        0x25E5 => rasterize_supersampled(buf, cw, ch, |x, y| ((w - x) / w) <= ((h - y) / h)),
        _ => return false,
    }
    true
}

fn draw_braille(buf: &mut [u8], cw: u32, ch: u32, bits: u8) {
    if bits == 0 {
        return;
    }
    let w = cw as f32;
    let h = ch as f32;
    let r = (w * 0.14).min(h * 0.08).max(0.85);
    let xs = [w * 0.30, w * 0.70];
    let ys = [h * 0.16, h * 0.38, h * 0.62, h * 0.84];
    // Braille dot bit mapping:
    // (col 0, row 0..3) -> bits 0, 1, 2, 6
    // (col 1, row 0..3) -> bits 3, 4, 5, 7
    const BIT_MAP: [[u8; 4]; 2] = [[0, 1, 2, 6], [3, 4, 5, 7]];
    rasterize_supersampled(buf, cw, ch, |x, y| {
        for (c, &cx) in xs.iter().enumerate() {
            for (row, &cy) in ys.iter().enumerate() {
                if (bits & (1 << BIT_MAP[c][row])) != 0
                    && (x - cx).powi(2) + (y - cy).powi(2) <= r * r {
                        return true;
                    }
            }
        }
        false
    });
}

fn draw_box_drawing(buf: &mut [u8], cw: u32, ch: u32, cp: u32) -> bool {
    let w = cw as f32;
    let h = ch as f32;
    let t_light = (w * 0.10).round().max(1.0) as u32;
    let t_heavy = (t_light * 2).max(2);
    let cx = cw / 2;
    let cy = ch / 2;

    // Diagonals U+2571..=U+2573
    match cp {
        0x2571 => {
            let hs = (t_light as f32 * 0.5).max(0.7);
            rasterize_supersampled(buf, cw, ch, |x, y| dist_to_segment(x, y, w, 0.0, 0.0, h) <= hs);
            return true;
        }
        0x2572 => {
            let hs = (t_light as f32 * 0.5).max(0.7);
            rasterize_supersampled(buf, cw, ch, |x, y| dist_to_segment(x, y, 0.0, 0.0, w, h) <= hs);
            return true;
        }
        0x2573 => {
            let hs = (t_light as f32 * 0.5).max(0.7);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, w, 0.0, 0.0, h) <= hs
                    || dist_to_segment(x, y, 0.0, 0.0, w, h) <= hs
            });
            return true;
        }
        // Rounded corners ╭ ╮ ╯ ╰ (U+256D..=U+2570)
        0x256D..=0x2570 => {
            let hs = (t_light as f32 * 0.5).max(0.65);
            let mcx = cx as f32 + 0.5;
            let mcy = cy as f32 + 0.5;
            let r = mcx.min(mcy).min(w - mcx).min(h - mcy).max(1.0);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let (ax_x, ax_y, sx, sy) = match cp {
                    0x256D => (mcx + r, mcy + r, 1.0f32, 1.0f32), // ╭ right + down
                    0x256E => (mcx - r, mcy + r, -1.0, 1.0),      // ╮ left + down
                    0x256F => (mcx - r, mcy - r, -1.0, -1.0),     // ╯ left + up
                    _ => (mcx + r, mcy - r, 1.0, -1.0),           // ╰ right + up
                };
                if (x - ax_x) * sx >= 0.0 && (y - mcy).abs() <= hs {
                    return true;
                }
                if (y - ax_y) * sy >= 0.0 && (x - mcx).abs() <= hs {
                    return true;
                }
                if (x - ax_x) * sx <= 0.0 && (y - ax_y) * sy <= 0.0 {
                    let d = ((x - ax_x).powi(2) + (y - ax_y).powi(2)).sqrt();
                    if (d - r).abs() <= hs {
                        return true;
                    }
                }
                false
            });
            return true;
        }
        _ => {}
    }

    // Arm weights: (left, right, up, down) where 0=none, 1=light, 2=heavy, 3=double
    let arms: (u8, u8, u8, u8) = match cp {
        // Horizontal & Vertical lines (including dashed variants)
        0x2500 | 0x2504 | 0x2508 | 0x254C => (1, 1, 0, 0),
        0x2501 | 0x2505 | 0x2509 | 0x254D => (2, 2, 0, 0),
        0x2502 | 0x2506 | 0x250A | 0x254E => (0, 0, 1, 1),
        0x2503 | 0x2507 | 0x250B | 0x254F => (0, 0, 2, 2),
        // Corners ┌ ┍ ┎ ┏ ┐ ┑ ┒ ㄱ └ ┕ ┖ ┗ ┘ ┙ ┚ ┛
        0x250C => (0, 1, 0, 1),
        0x250D => (0, 2, 0, 1),
        0x250E => (0, 1, 0, 2),
        0x250F => (0, 2, 0, 2),
        0x2510 => (1, 0, 0, 1),
        0x2511 => (2, 0, 0, 1),
        0x2512 => (1, 0, 0, 2),
        0x2513 => (2, 0, 0, 2),
        0x2514 => (0, 1, 1, 0),
        0x2515 => (0, 2, 1, 0),
        0x2516 => (0, 1, 2, 0),
        0x2517 => (0, 2, 2, 0),
        0x2518 => (1, 0, 1, 0),
        0x2519 => (2, 0, 1, 0),
        0x251A => (1, 0, 2, 0),
        0x251B => (2, 0, 2, 0),
        // T-junctions ├..=┫, ┬..=┳, ┴..=┻
        0x251C => (0, 1, 1, 1),
        0x251D => (0, 2, 1, 1),
        0x251E => (0, 1, 2, 1),
        0x251F => (0, 1, 1, 2),
        0x2520 => (0, 1, 2, 2),
        0x2521 => (0, 2, 2, 1),
        0x2522 => (0, 2, 1, 2),
        0x2523 => (0, 2, 2, 2),
        0x2524 => (1, 0, 1, 1),
        0x2525 => (2, 0, 1, 1),
        0x2526 => (1, 0, 2, 1),
        0x2527 => (1, 0, 1, 2),
        0x2528 => (1, 0, 2, 2),
        0x2529 => (2, 0, 2, 1),
        0x252A => (2, 0, 1, 2),
        0x252B => (2, 0, 2, 2),
        0x252C => (1, 1, 0, 1),
        0x252D => (2, 1, 0, 1),
        0x252E => (1, 2, 0, 1),
        0x252F => (2, 2, 0, 1),
        0x2530 => (1, 1, 0, 2),
        0x2531 => (2, 1, 0, 2),
        0x2532 => (1, 2, 0, 2),
        0x2533 => (2, 2, 0, 2),
        0x2534 => (1, 1, 1, 0),
        0x2535 => (2, 1, 1, 0),
        0x2536 => (1, 2, 1, 0),
        0x2537 => (2, 2, 1, 0),
        0x2538 => (1, 1, 2, 0),
        0x2539 => (2, 1, 2, 0),
        0x253A => (1, 2, 2, 0),
        0x253B => (2, 2, 2, 0),
        // Crosses ┼..=╋
        0x253C => (1, 1, 1, 1),
        0x253D => (2, 1, 1, 1),
        0x253E => (1, 2, 1, 1),
        0x253F => (2, 2, 1, 1),
        0x2540 => (1, 1, 2, 1),
        0x2541 => (1, 1, 1, 2),
        0x2542 => (1, 1, 2, 2),
        0x2543 => (2, 1, 2, 1),
        0x2544 => (1, 2, 2, 1),
        0x2545 => (2, 1, 1, 2),
        0x2546 => (1, 2, 1, 2),
        0x2547 => (2, 2, 2, 1),
        0x2548 => (2, 2, 1, 2),
        0x2549 => (2, 1, 2, 2),
        0x254A => (1, 2, 2, 2),
        0x254B => (2, 2, 2, 2),
        // Double lines & mixed double/single U+2550..=U+256C
        0x2550 => (3, 3, 0, 0),
        0x2551 => (0, 0, 3, 3),
        0x2552 => (0, 3, 0, 1),
        0x2553 => (0, 1, 0, 3),
        0x2554 => (0, 3, 0, 3),
        0x2555 => (3, 0, 0, 1),
        0x2556 => (1, 0, 0, 3),
        0x2557 => (3, 0, 0, 3),
        0x2558 => (0, 3, 1, 0),
        0x2559 => (0, 1, 3, 0),
        0x255A => (0, 3, 3, 0),
        0x255B => (3, 0, 1, 0),
        0x255C => (1, 0, 3, 0),
        0x255D => (3, 0, 3, 0),
        0x255E => (0, 3, 1, 1),
        0x255F => (0, 1, 3, 3),
        0x2560 => (0, 3, 3, 3),
        0x2561 => (3, 0, 1, 1),
        0x2562 => (1, 0, 3, 3),
        0x2563 => (3, 0, 3, 3),
        0x2564 => (3, 3, 0, 1),
        0x2565 => (1, 1, 0, 3),
        0x2566 => (3, 3, 0, 3),
        0x2567 => (3, 3, 1, 0),
        0x2568 => (1, 1, 3, 0),
        0x2569 => (3, 3, 3, 0),
        0x256A => (3, 3, 1, 1),
        0x256B => (1, 1, 3, 3),
        0x256C => (3, 3, 3, 3),
        // Half-lines U+2574..=U+257F
        0x2574 => (1, 0, 0, 0),
        0x2575 => (0, 0, 1, 0),
        0x2576 => (0, 1, 0, 0),
        0x2577 => (0, 0, 0, 1),
        0x2578 => (2, 0, 0, 0),
        0x2579 => (0, 0, 2, 0),
        0x257A => (0, 2, 0, 0),
        0x257B => (0, 0, 0, 2),
        0x257C => (1, 2, 0, 0),
        0x257D => (0, 0, 1, 2),
        0x257E => (2, 1, 0, 0),
        0x257F => (0, 0, 2, 1),
        _ => return false,
    };

    let (l, r, u, d) = arms;
    let span = |center: u32, t: u32| -> (u32, u32) {
        let s = center.saturating_sub(t / 2);
        (s, s + t)
    };
    let double_off = t_light.max(1) + (t_light / 2).max(1);

    let draw_h = |buf: &mut [u8], x0: u32, x1: u32, style: u8| {
        match style {
            1 => {
                let (y0, y1) = span(cy, t_light);
                fill_rect(buf, cw, ch, x0, y0, x1, y1, 255);
            }
            2 => {
                let (y0, y1) = span(cy, t_heavy);
                fill_rect(buf, cw, ch, x0, y0, x1, y1, 255);
            }
            3 => {
                let (ya0, ya1) = span(cy.saturating_sub(double_off), t_light);
                let (yb0, yb1) = span(cy + double_off, t_light);
                fill_rect(buf, cw, ch, x0, ya0, x1, ya1, 255);
                fill_rect(buf, cw, ch, x0, yb0, x1, yb1, 255);
            }
            _ => {}
        }
    };
    let draw_v = |buf: &mut [u8], y0: u32, y1: u32, style: u8| {
        match style {
            1 => {
                let (x0, x1) = span(cx, t_light);
                fill_rect(buf, cw, ch, x0, y0, x1, y1, 255);
            }
            2 => {
                let (x0, x1) = span(cx, t_heavy);
                fill_rect(buf, cw, ch, x0, y0, x1, y1, 255);
            }
            3 => {
                let (xa0, xa1) = span(cx.saturating_sub(double_off), t_light);
                let (xb0, xb1) = span(cx + double_off, t_light);
                fill_rect(buf, cw, ch, xa0, y0, xa1, y1, 255);
                fill_rect(buf, cw, ch, xb0, y0, xb1, y1, 255);
            }
            _ => {}
        }
    };

    let pad_c = (t_heavy / 2).max(1);
    if l != 0 {
        draw_h(buf, 0, (cx + pad_c).min(cw), l);
    }
    if r != 0 {
        draw_h(buf, cx.saturating_sub(pad_c), cw, r);
    }
    if u != 0 {
        draw_v(buf, 0, (cy + pad_c).min(ch), u);
    }
    if d != 0 {
        draw_v(buf, cy.saturating_sub(pad_c), ch, d);
    }
    true
}

/// Built-in vector fallback for common Nerd Font PUA prompt icons when no Nerd Font is installed.
pub(super) fn draw_nerd_icon_fallback(buf: &mut [u8], cw: u32, ch: u32, cp: u32) -> bool {
    let w = cw as f32;
    let h = ch as f32;
    match cp {
        // Git branch / Git icons in Nerd Fonts (fa-code-fork, dev-git_branch, dev-git)
        0xF126 | 0xE725 | 0xE702 | 0xF0620 => draw_powerline(buf, cw, ch, 0xE0A0),
        // Lock icons in Nerd Fonts (fa-lock)
        0xF023 | 0xF033E => draw_powerline(buf, cw, ch, 0xE0A2),
        // Folder icons in Nerd Fonts (fa-folder, fa-folder-open, custom-folder)
        0xF07B | 0xF07C | 0xE5FF | 0xE5FE | 0xF024B => {
            rasterize_supersampled(buf, cw, ch, |x, y| {
                let in_body = x >= w * 0.12 && x <= w * 0.88 && y >= h * 0.34 && y <= h * 0.78;
                let in_tab = x >= w * 0.12 && x <= w * 0.46 && y >= h * 0.24 && y <= h * 0.36;
                in_body || in_tab
            });
            true
        }
        // Checkmark icons (fa-check)
        0xF00C | 0x2713 | 0x2714 => {
            let sw = (w * 0.09).max(0.85);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, w * 0.18, h * 0.54, w * 0.42, h * 0.74) <= sw
                    || dist_to_segment(x, y, w * 0.42, h * 0.74, w * 0.84, h * 0.28) <= sw
            });
            true
        }
        // Cross / X icons (fa-times / fa-close)
        0xF00D | 0x2717 | 0x2718 => {
            let sw = (w * 0.09).max(0.85);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, w * 0.22, h * 0.28, w * 0.78, h * 0.72) <= sw
                    || dist_to_segment(x, y, w * 0.78, h * 0.28, w * 0.22, h * 0.72) <= sw
            });
            true
        }
        // Heavy right-pointing angle bracket ❯ (Starship / Pure prompt) & Nerd Font chevron-right
        0x276F | 0xF054 => {
            let sw = (w * 0.10).max(0.9);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, w * 0.26, h * 0.24, w * 0.74, h * 0.50) <= sw
                    || dist_to_segment(x, y, w * 0.74, h * 0.50, w * 0.26, h * 0.76) <= sw
            });
            true
        }
        // Heavy right-pointing arrow ➜ (Oh-My-Zsh robbyrussell prompt) & Nerd Font arrow-right
        0x279C | 0xF061 | 0xF178 => {
            let sw = (w * 0.095).max(0.85);
            rasterize_supersampled(buf, cw, ch, |x, y| {
                dist_to_segment(x, y, w * 0.14, h * 0.50, w * 0.82, h * 0.50) <= sw
                    || dist_to_segment(x, y, w * 0.52, h * 0.28, w * 0.84, h * 0.50) <= sw
                    || dist_to_segment(x, y, w * 0.84, h * 0.50, w * 0.52, h * 0.72) <= sw
            });
            true
        }
        _ => false,
    }
}
