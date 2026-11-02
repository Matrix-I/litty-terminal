//! Terminal emulation core: the VT parser and cell grid, the PTY, scrollback
//! search, shell history, bidi, inline graphics and shell integration.
//!
//! Layer rule (enforced by the `layering_tests` module): nothing in here uses a
//! crate module outside `terminal`.

pub(crate) mod bidi;
pub(crate) mod graphics;
pub(crate) mod grid;
pub(crate) mod history;
pub(crate) mod pty;
pub(crate) mod search;
pub(crate) mod shell_integration;
pub(crate) mod text;
pub(crate) mod x11colors;
