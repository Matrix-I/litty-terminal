//! CoreGraphics geometry structs shared by the CoreText glyph engine
//! (`atlas/coretext.rs`) and the Metal backend (`backend/metal`).
//!
//! The platform layer keeps its own `NSRect` / `NSPoint` (layout-identical):
//! the layer rules forbid `platform` from using `render`.

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

// CGFloat is `double` on 64-bit Apple targets; these pass by value through FFI.
const _: () = assert!(std::mem::size_of::<CGPoint>() == 16 && std::mem::size_of::<CGSize>() == 16);
const _: () = assert!(std::mem::size_of::<CGRect>() == 32);
