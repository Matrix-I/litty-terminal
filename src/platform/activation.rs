//! Bounded activation messages. No terminal or application dependencies.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use std::io::{self, Read, Write};

pub const MAX_CWD: usize = 4096;
const HEADER_LEN: usize = 49; // magic(4), id(16), session(16), expiry(8), action(1), size(4)
const MAX_OUTCOMES: usize = 64;
pub const MAX_FRAME: usize = 16 * 1024;
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationAction {
    NewWindow(String),
    Pane(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationRequest {
    pub id: [u8; 16],
    pub session: [u8; 16],
    pub expires_ns: u64,
    pub action: ActivationAction,
}

pub type ActivationReply = Result<(), String>;
pub type ActivationSink = std::sync::Arc<
    dyn Fn(ActivationRequest, std::sync::mpsc::SyncSender<ActivationReply>) + Send + Sync,
>;

pub fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid activation message")
}

pub fn validate_cwd(cwd: &str) -> io::Result<&str> {
    if cwd.len() > MAX_CWD
        || cwd.as_bytes().contains(&0)
        || !std::path::Path::new(cwd).is_absolute()
    {
        return Err(invalid());
    }
    Ok(cwd)
}

impl ActivationRequest {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let (kind, payload) = match &self.action {
            ActivationAction::NewWindow(cwd) => (0, validate_cwd(cwd)?.as_bytes().to_vec()),
            ActivationAction::Pane(id) if *id != 0 => (1, id.to_be_bytes().to_vec()),
            _ => return Err(invalid()),
        };
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
        out.extend_from_slice(b"LTA1");
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&self.session);
        out.extend_from_slice(&self.expires_ns.to_be_bytes());
        out.push(kind);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < HEADER_LEN || bytes.len() > MAX_FRAME || &bytes[..4] != b"LTA1" {
            return Err(invalid());
        }
        let len = u32::from_be_bytes(bytes[45..49].try_into().map_err(|_| invalid())?) as usize;
        if len != bytes.len() - HEADER_LEN {
            return Err(invalid());
        }
        let action = match bytes[44] {
            0 => ActivationAction::NewWindow(
                validate_cwd(std::str::from_utf8(&bytes[49..]).map_err(|_| invalid())?)?.to_owned(),
            ),
            1 if len == 8 => {
                let id = u64::from_be_bytes(bytes[49..].try_into().map_err(|_| invalid())?);
                if id == 0 {
                    return Err(invalid());
                }
                ActivationAction::Pane(id)
            }
            _ => return Err(invalid()),
        };
        Ok(Self {
            id: bytes[4..20].try_into().map_err(|_| invalid())?,
            session: bytes[20..36].try_into().map_err(|_| invalid())?,
            expires_ns: u64::from_be_bytes(bytes[36..44].try_into().map_err(|_| invalid())?),
            action,
        })
    }
}

pub fn write_frame(mut out: impl Write, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(invalid());
    }
    out.write_all(&(bytes.len() as u32).to_be_bytes())?;
    out.write_all(bytes)
}

pub fn read_frame(mut input: impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    input.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size > MAX_FRAME {
        return Err(invalid());
    }
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Entries survive their entire replay horizon. Saturation rejects admission;
/// an old ID can never be evicted while its original request remains valid.
#[derive(Default)]
pub struct Outcomes(Vec<(ActivationRequest, ActivationReply)>);
impl Outcomes {
    pub fn lookup(
        &mut self,
        req: &ActivationRequest,
        now: u64,
    ) -> io::Result<Option<ActivationReply>> {
        self.0.retain(|(r, _)| r.expires_ns > now);
        if req.expires_ns <= now || req.expires_ns - now > TIMEOUT.as_nanos() as u64 {
            return Err(invalid());
        }
        if let Some((old, result)) = self.0.iter().find(|(r, _)| r.id == req.id) {
            return if old == req {
                Ok(Some(result.clone()))
            } else {
                Err(invalid())
            };
        }
        if self.0.len() >= MAX_OUTCOMES {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "activation replay cache full",
            ));
        }
        Ok(None)
    }
    pub fn record(&mut self, req: ActivationRequest, reply: ActivationReply) {
        self.0.push((req, reply));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(action: ActivationAction) -> ActivationRequest {
        ActivationRequest {
            id: [1; 16],
            session: [2; 16],
            expires_ns: 200,
            action,
        }
    }
    #[test]
    fn wire_roundtrips_and_rejects_malformed_frames() {
        for action in [
            ActivationAction::NewWindow("/tmp/tiếng việt".into()),
            ActivationAction::Pane(42),
        ] {
            let r = request(action);
            let encoded = r.encode().unwrap();
            assert_eq!(ActivationRequest::decode(&encoded).unwrap(), r);
            for cut in 0..encoded.len() {
                assert!(ActivationRequest::decode(&encoded[..cut]).is_err());
            }
            let mut extra = encoded.clone();
            extra.push(0);
            assert!(ActivationRequest::decode(&extra).is_err());
            let mut version = encoded;
            version[3] = b'2';
            assert!(ActivationRequest::decode(&version).is_err());
        }
        assert!(ActivationRequest::decode(&vec![0; MAX_FRAME + 1]).is_err());
        assert!(request(ActivationAction::Pane(0)).encode().is_err());
    }
    #[test]
    fn cwd_boundary_preserves_unicode_and_rejects_invalid_paths() {
        for cwd in [
            "relative",
            "",
            "/tmp/\0bad",
            &format!("/{}", "x".repeat(MAX_CWD)),
        ] {
            assert!(validate_cwd(cwd).is_err());
        }
        assert_eq!(validate_cwd("/tmp/tiếng việt").unwrap(), "/tmp/tiếng việt");
    }
    #[test]
    fn replay_cache_never_evicts_a_valid_id() {
        let mut cache = Outcomes::default();
        let r = request(ActivationAction::Pane(1));
        assert_eq!(cache.lookup(&r, 100).unwrap(), None);
        cache.record(r.clone(), Ok(()));
        assert_eq!(cache.lookup(&r, 100).unwrap(), Some(Ok(())));
        let changed = request(ActivationAction::Pane(2));
        assert!(cache.lookup(&changed, 100).is_err());
        for i in 2..=64 {
            let mut next = r.clone();
            next.id[0] = i;
            cache.record(next, Ok(()));
        }
        let mut overflow = r.clone();
        overflow.id[0] = 65;
        assert_eq!(
            cache.lookup(&overflow, 100).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(cache.lookup(&r, 100).unwrap(), Some(Ok(())));
        assert!(cache.lookup(&r, 200).is_err());
        overflow.expires_ns = 300;
        assert_eq!(cache.lookup(&overflow, 200).unwrap(), None);
    }
    #[test]
    fn framed_io_bounds_allocations_and_detects_truncation() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"ok").unwrap();
        assert_eq!(read_frame(bytes.as_slice()).unwrap(), b"ok");
        assert!(read_frame(&bytes[..bytes.len() - 1]).is_err());
        assert!(read_frame(((MAX_FRAME + 1) as u32).to_be_bytes().as_slice()).is_err());
    }
}
