//! The monitor under an X11 window, through a dynamically loaded
//! `libXrandr.so.2` (CRTC geometry and the current mode's refresh rate).

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::sync::OnceLock;

use crate::platform::gui::{Monitor, PhysicalPosition, PhysicalSize};
use super::sym;

/// `RR_Interlace` and `RR_DoubleScan` mode flags (`<X11/extensions/randr.h>`).
const RR_INTERLACE: c_ulong = 0x10;
const RR_DOUBLE_SCAN: c_ulong = 0x20;

#[repr(C)]
struct XRRModeInfo {
    id: c_ulong,
    width: c_uint,
    height: c_uint,
    dot_clock: c_ulong,
    h_sync_start: c_uint,
    h_sync_end: c_uint,
    h_total: c_uint,
    h_skew: c_uint,
    v_sync_start: c_uint,
    v_sync_end: c_uint,
    v_total: c_uint,
    name: *mut c_char,
    name_length: c_uint,
    mode_flags: c_ulong,
}

#[repr(C)]
struct XRRScreenResources {
    timestamp: c_ulong,
    config_timestamp: c_ulong,
    ncrtc: c_int,
    crtcs: *mut c_ulong,
    noutput: c_int,
    outputs: *mut c_ulong,
    nmode: c_int,
    modes: *mut XRRModeInfo,
}

#[repr(C)]
struct XRRCrtcInfo {
    timestamp: c_ulong,
    x: c_int,
    y: c_int,
    width: c_uint,
    height: c_uint,
    mode: c_ulong,
    rotation: u16,
    noutput: c_int,
    outputs: *mut c_ulong,
    rotations: u16,
    npossible: c_int,
    possible: *mut c_ulong,
}

#[allow(non_snake_case)]
struct XrandrLib {
    XRRGetScreenResourcesCurrent: unsafe extern "C" fn(*mut c_void, c_ulong) -> *mut XRRScreenResources,
    XRRFreeScreenResources: unsafe extern "C" fn(*mut XRRScreenResources),
    XRRGetCrtcInfo: unsafe extern "C" fn(*mut c_void, *mut XRRScreenResources, c_ulong) -> *mut XRRCrtcInfo,
    XRRFreeCrtcInfo: unsafe extern "C" fn(*mut XRRCrtcInfo),
}

// Plain function pointers into a library that is never unloaded.
unsafe impl Send for XrandrLib {}
unsafe impl Sync for XrandrLib {}

fn xrandr() -> Option<&'static XrandrLib> {
    static LIB: OnceLock<Option<XrandrLib>> = OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let handle = libc::dlopen(c"libXrandr.so.2".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if handle.is_null() {
            return None;
        }
        Some(XrandrLib {
            XRRGetScreenResourcesCurrent: sym(handle, c"XRRGetScreenResourcesCurrent")?,
            XRRFreeScreenResources: sym(handle, c"XRRFreeScreenResources")?,
            XRRGetCrtcInfo: sym(handle, c"XRRGetCrtcInfo")?,
            XRRFreeCrtcInfo: sym(handle, c"XRRFreeCrtcInfo")?,
        })
    })
    .as_ref()
}

/// Refresh rate of a mode in millihertz, as `xrandr` computes it; `None`
/// for a mode without timings.
fn refresh_millihertz(dot_clock: u64, h_total: u32, v_total: u32, flags: c_ulong) -> Option<u32> {
    // Double scan draws each line twice; interlacing shows half the lines
    // per field (the numerator doubles instead, as 1125 lines is odd).
    let mut pixels = u64::from(h_total) * u64::from(v_total);
    let mut millihertz = dot_clock.checked_mul(1000)?;
    if flags & RR_DOUBLE_SCAN != 0 {
        pixels *= 2;
    }
    if flags & RR_INTERLACE != 0 {
        millihertz = millihertz.checked_mul(2)?;
    }
    if pixels == 0 {
        return None;
    }
    u32::try_from((millihertz + pixels / 2) / pixels).ok()
}

/// The monitor holding `point`; outside every monitor (a window dragged
/// off-screen), the first one.
fn monitor_at(monitors: &[Monitor], point: PhysicalPosition<i32>) -> Option<Monitor> {
    let contains = |m: &&Monitor| {
        let (x, y) = (i64::from(point.x), i64::from(point.y));
        let (left, top) = (i64::from(m.position.x), i64::from(m.position.y));
        (left..left + i64::from(m.size.width)).contains(&x) && (top..top + i64::from(m.size.height)).contains(&y)
    };
    monitors.iter().find(contains).or(monitors.first()).cloned()
}

/// Active CRTCs of the screen with `root`, in X order.
///
/// # Safety
/// `display` and `root` must be live handles.
unsafe fn active_monitors(lib: &XrandrLib, display: *mut c_void, root: c_ulong) -> Vec<Monitor> {
    let mut monitors = Vec::new();
    unsafe {
        let res = (lib.XRRGetScreenResourcesCurrent)(display, root);
        if res.is_null() {
            return monitors;
        }
        let r = &*res;
        let crtcs: &[c_ulong] = if r.crtcs.is_null() { &[] } else { std::slice::from_raw_parts(r.crtcs, r.ncrtc.max(0) as usize) };
        let modes: &[XRRModeInfo] = if r.modes.is_null() { &[] } else { std::slice::from_raw_parts(r.modes, r.nmode.max(0) as usize) };
        for &crtc in crtcs {
            let info = (lib.XRRGetCrtcInfo)(display, res, crtc);
            if info.is_null() {
                continue;
            }
            let c = &*info;
            // A CRTC without a mode drives no output.
            if c.mode != 0 && c.width > 0 && c.height > 0 {
                let refresh = modes
                    .iter()
                    .find(|m| m.id == c.mode)
                    .and_then(|m| refresh_millihertz(m.dot_clock, m.h_total, m.v_total, m.mode_flags));
                monitors.push(Monitor {
                    position: PhysicalPosition::new(c.x, c.y),
                    size: PhysicalSize::new(c.width, c.height),
                    refresh_rate_millihertz: refresh,
                });
            }
            (lib.XRRFreeCrtcInfo)(info);
        }
        (lib.XRRFreeScreenResources)(res);
    }
    monitors
}

/// The monitor showing `point` (root coordinates); `None` without RandR.
///
/// # Safety
/// `display` and `root` must be live handles.
pub unsafe fn monitor_for_point(display: *mut c_void, root: c_ulong, point: PhysicalPosition<i32>) -> Option<Monitor> {
    let lib = xrandr()?;
    monitor_at(&unsafe { active_monitors(lib, display, root) }, point)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3072x1728 at 451.75 MHz (a real laptop panel): `xrandr` rounds it to 59.94 Hz.
    #[test]
    fn refresh_rate_comes_from_the_mode_timings() {
        assert_eq!(refresh_millihertz(451_750_000, 4208, 1791, 0), Some(59_941));
        assert_eq!(refresh_millihertz(148_500_000, 2200, 1125, 0), Some(60_000));
    }

    #[test]
    fn interlaced_and_double_scan_modes_adjust_the_frame_count() {
        // 1080i: half the lines per field, twice the fields.
        assert_eq!(refresh_millihertz(74_250_000, 2200, 1125, RR_INTERLACE), Some(60_000));
        assert_eq!(refresh_millihertz(148_500_000, 2200, 1125, RR_DOUBLE_SCAN), Some(30_000));
    }

    #[test]
    fn modes_without_timings_have_no_refresh_rate() {
        assert_eq!(refresh_millihertz(148_500_000, 0, 1125, 0), None);
        assert_eq!(refresh_millihertz(148_500_000, 2200, 0, 0), None);
    }

    fn monitor(x: i32, y: i32, w: u32, h: u32, hz: u32) -> Monitor {
        Monitor {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(w, h),
            refresh_rate_millihertz: Some(hz),
        }
    }

    #[test]
    fn the_monitor_under_the_point_is_chosen() {
        let monitors = [monitor(0, 0, 2560, 1440, 60_000), monitor(2560, 0, 1920, 1080, 144_000)];
        assert_eq!(monitor_at(&monitors, PhysicalPosition::new(3000, 500)), Some(monitors[1].clone()));
        assert_eq!(monitor_at(&monitors, PhysicalPosition::new(2559, 1439)), Some(monitors[0].clone()));
        // Right and bottom edges belong to the next monitor or to none.
        assert_eq!(monitor_at(&monitors, PhysicalPosition::new(2560, 0)), Some(monitors[1].clone()));
    }

    #[test]
    fn a_point_off_every_monitor_falls_back_to_the_first() {
        let monitors = [monitor(0, 0, 1920, 1080, 60_000), monitor(-1280, 0, 1280, 1024, 75_000)];
        assert_eq!(monitor_at(&monitors, PhysicalPosition::new(-1000, 200)), Some(monitors[1].clone()));
        assert_eq!(monitor_at(&monitors, PhysicalPosition::new(5000, 5000)), Some(monitors[0].clone()));
        assert_eq!(monitor_at(&[], PhysicalPosition::new(0, 0)), None);
    }
}
