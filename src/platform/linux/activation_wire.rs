//! Linux activation frames. `LTA2` carries an XDG activation token ahead of
//! an unchanged `LTA1` request, so a running instance can focus the window
//! it opens; plain `LTA1` frames still decode.
use crate::platform::activation::{self as wire, ActivationRequest};
use super::native::wayland_activation::valid_token;
use std::io;

const MAGIC: &[u8; 4] = b"LTA2";

/// `request` as a frame, with `token` ahead of it when there is one.
pub(super) fn encode(request: &ActivationRequest, token: Option<&str>) -> io::Result<Vec<u8>> {
    let inner = request.encode()?;
    let Some(token) = token else { return Ok(inner) };
    if !valid_token(token) {
        return Err(wire::invalid());
    }
    // `valid_token` bounds a token to 4096 bytes, so its length fits a u16.
    let mut out = Vec::with_capacity(MAGIC.len() + 2 + token.len() + inner.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(token.len() as u16).to_be_bytes());
    out.extend_from_slice(token.as_bytes());
    out.extend_from_slice(&inner);
    Ok(out)
}

/// The request in an `LTA1` or `LTA2` frame, and the `LTA2` token.
pub(super) fn decode(bytes: &[u8]) -> io::Result<(ActivationRequest, Option<String>)> {
    let Some(rest) = bytes.strip_prefix(MAGIC) else {
        return Ok((ActivationRequest::decode(bytes)?, None));
    };
    let (len, rest) = rest.split_first_chunk::<2>().ok_or_else(wire::invalid)?;
    let len = usize::from(u16::from_be_bytes(*len));
    if rest.len() < len {
        return Err(wire::invalid());
    }
    let (token, inner) = rest.split_at(len);
    let token = std::str::from_utf8(token).map_err(|_| wire::invalid())?;
    if !valid_token(token) {
        return Err(wire::invalid());
    }
    // `ActivationRequest::decode` accepts only a whole `LTA1` frame: a nested
    // `LTA2` or trailing bytes are rejected there.
    Ok((ActivationRequest::decode(inner)?, Some(token.to_owned())))
}

