//! Inline images: the Kitty graphics protocol (`ESC _ G ... ESC \`) and Sixel
//! (`ESC P q ... ESC \`). Images are decoded to RGBA and kept per terminal;
//! placements are anchored to absolute lines, so they scroll with the text.
//! The renderer uploads each image once and draws its placements on top of
//! the cells.

mod animation;
mod kitty;
pub(crate) mod png;
mod sixel;
mod store;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};

/// Decoded images kept per terminal, in bytes of RGBA; the oldest go first.
pub(crate) const STORE_LIMIT: usize = 64 << 20;
/// Independent object cap: tiny transmit-only images still allocate map entries,
/// Arc headers, insertion-order slots, and animation metadata.
const MAX_IMAGES: usize = 1024;
/// Largest image accepted, per side.
const MAX_SIDE: u32 = 8192;
/// Largest decoded image, in pixels: its RGBA must fit in the store.
const MAX_PIXELS: usize = STORE_LIMIT / 4;
/// Regular placements kept per terminal; the oldest go first.
const MAX_PLACEMENTS: usize = 4096;
/// Virtual (`U=1`) placements kept per terminal; the oldest go first.
const MAX_VIRTUALS: usize = 1024;
/// Maximum number of additional animation frames per image.
const MAX_ANIMATION_FRAMES: usize = 256;
/// Maximum bytes allocated to a single animated image.
const MAX_ANIMATION_BYTES: usize = STORE_LIMIT / 2;

/// Process-wide image keys, so the renderer's texture cache never confuses
/// images of different terminals.
fn next_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub use animation::any_animation;
pub(crate) use kitty::{decode_kitty, DecodedImage, KittyPreparation};
pub use kitty::{parse_kitty, KittyCommand, KittyOutcome};
#[cfg(test)]
pub use png::decode_png;
pub use sixel::decode_sixel;
pub use store::{ImageDraw, Placement, Store, VirtualPlacement, PLACEHOLDER_DIACRITICS, Z_BELOW_BG};
