//! Integrity and authenticity of a download: SHA-256 checksums and pinned-key signatures.

use std::fs;
use std::path::Path;
use std::process::Command;

use super::{ReleaseAsset, ReleaseInfo, SUMS_NAME};

pub(super) const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Standalone, pure-Rust streaming SHA-256 implementation (FIPS 180-4).
/// Completely allocation-free block compression; zero external dependencies.
#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn compress_block(h: &mut [u32; 8], chunk: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, part) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([part[0], part[1], part[2], part[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut h_val = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h_val.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h_val = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(h_val);
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;

        if self.buf_len > 0 {
            let to_fill = 64 - self.buf_len;
            if data.len() < to_fill {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..64].copy_from_slice(&data[..to_fill]);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf_len = 0;
            data = &data[to_fill..];
        }

        while data.len() >= 64 {
            let chunk: &[u8; 64] = data[..64].try_into().expect("64-byte chunk");
            Self::compress_block(&mut self.h, chunk);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    pub fn finalize(mut self) -> String {
        let bit_len = self.total_len.wrapping_mul(8);
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;

        if self.buf_len > 56 {
            self.buf[self.buf_len..64].fill(0);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf_len = 0;
        }

        self.buf[self.buf_len..56].fill(0);
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        Self::compress_block(&mut self.h, &self.buf);

        let mut res = String::with_capacity(64);
        for val in self.h {
            res.push_str(&format!("{val:08x}"));
        }
        res
    }
}

#[allow(dead_code)]
pub fn sha256_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = fs::File::open(path).map_err(|e| format!("cannot open file for hashing: {e}"))?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 65536];
    loop {
        let n = file.read(&mut chunk).map_err(|e| format!("cannot read file for hashing: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    Ok(hasher.finalize())
}

/// Expected SHA-256 of `asset_name` in a `SHA256SUMS.txt` (`<hash>  [*]<name>` lines).
pub(super) fn expected_hash(sums: &str, asset_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset_name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// Checksum validation against the contents of `SHA256SUMS.txt`.
pub(super) fn verify_checksum(sums: &str, asset_name: &str, downloaded_file: &Path) -> Result<(), String> {
    let expected =
        expected_hash(sums, asset_name).ok_or_else(|| format!("asset `{asset_name}` not found in {SUMS_NAME}"))?;
    let actual = sha256_file(downloaded_file)?;

    if expected != actual {
        return Err(format!(
            "checksum mismatch for {asset_name}!\nExpected: {expected}\nActual:   {actual}"
        ));
    }
    Ok(())
}

/// Release signing keys, baked in at build time (never read from the runtime
/// environment, where any process that can set variables could swap them):
///
/// ```text
/// LITTY_MINISIGN_PUBKEY=RWQ...  LITTY_GPG_FINGERPRINT=ABCD...  cargo build --release
/// ```
///
/// A build without a key cannot verify releases signed with that tool and
/// refuses to install them (fail closed).
pub const MINISIGN_PUBKEY: Option<&str> = option_env!("LITTY_MINISIGN_PUBKEY");
/// Full 40-hex-digit fingerprint of the release GPG key (spaces allowed).
pub const GPG_FINGERPRINT: Option<&str> = option_env!("LITTY_GPG_FINGERPRINT");

/// Signature assets looked for next to `SHA256SUMS.txt`, in order of preference.
pub(super) const SIG_NAMES: [&str; 3] = ["SHA256SUMS.txt.minisig", "SHA256SUMS.txt.sig", "SHA256SUMS.txt.asc"];

pub(super) fn normalize_fingerprint(fpr: &str) -> String {
    fpr.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_uppercase()
}

/// The release's signature over `SHA256SUMS.txt`: one this build has a pinned
/// key for if possible, else any (which then fails verification, closed).
pub(super) fn signature_asset(release: &ReleaseInfo) -> Option<&ReleaseAsset> {
    let has_key = |name: &str| {
        if name.ends_with(".minisig") {
            MINISIGN_PUBKEY.is_some()
        } else {
            GPG_FINGERPRINT.is_some()
        }
    };
    let find = |name: &str| release.assets.iter().find(|a| a.name == name);
    SIG_NAMES.iter().filter(|n| has_key(n)).find_map(|n| find(n)).or_else(|| SIG_NAMES.iter().find_map(|n| find(n)))
}

/// Cryptographic signature verification (Minisign or GPG) with key pinning.
/// Fails closed: a missing verifier, a missing pinned key or an unknown
/// signature type is an error, never a silent pass.
pub(crate) fn verify_signature(sums_path: &Path, sig_path: &Path) -> Result<(), String> {
    let ext = sig_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let find_bin = |name: &str| {
        ["/usr/bin/", "/usr/local/bin/", "/opt/homebrew/bin/"]
            .into_iter()
            .map(|dir| format!("{dir}{name}"))
            .find(|b| Path::new(b).is_file())
    };

    if ext.eq_ignore_ascii_case("minisig") {
        let pubkey = MINISIGN_PUBKEY
            .filter(|k| !k.trim().is_empty())
            .ok_or("this build has no pinned minisign key; refusing an unverifiable update")?;
        let bin = find_bin("minisign")
            .ok_or("minisign is required to verify this update (install it, e.g. `brew install minisign`)")?;
        let status = Command::new(bin)
            .args(["-V", "-q", "-m"])
            .arg(sums_path)
            .arg("-x")
            .arg(sig_path)
            .arg("-P")
            .arg(pubkey.trim())
            .status()
            .map_err(|e| format!("failed to execute minisign: {e}"))?;
        if !status.success() {
            return Err(format!("minisign signature verification failed (exit code: {:?})", status.code()));
        }
        Ok(())
    } else if ext.eq_ignore_ascii_case("sig") || ext.eq_ignore_ascii_case("asc") {
        let pinned = GPG_FINGERPRINT
            .map(normalize_fingerprint)
            .filter(|f| f.len() == 40 && f.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("this build has no pinned GPG key fingerprint; refusing an unverifiable update")?;
        let bin = find_bin("gpg").ok_or("gpg is required to verify this update")?;
        let out = Command::new(bin)
            .args(["--batch", "--no-tty", "--status-fd", "1", "--verify"])
            .arg(sig_path)
            .arg(sums_path)
            .stderr(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("failed to execute GPG: {e}"))?;
        // A good signature from *any* key in the user's keyring is not enough:
        // require VALIDSIG from the pinned key (signing subkey or primary).
        let status = String::from_utf8_lossy(&out.stdout);
        let pinned_ok = status.lines().any(|line| {
            let mut f = line.split_whitespace();
            f.next() == Some("[GNUPG:]")
                && f.next() == Some("VALIDSIG")
                && line.split_whitespace().skip(2).any(|tok| normalize_fingerprint(tok) == pinned)
        });
        if !out.status.success() || !pinned_ok {
            return Err(format!(
                "GPG signature verification failed (exit code: {:?}, pinned key {})",
                out.status.code(),
                if pinned_ok { "matched" } else { "not matched" }
            ));
        }
        Ok(())
    } else {
        Err(format!("unsupported signature file `{}`", sig_path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::temp_dir;

    #[test]
    fn test_sha256_nist_vectors() {
        // Standard NIST test vectors
        assert_eq!(
            sha256_digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn test_sha256_file() {
        let dir = temp_dir("sha256_file");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hash_test.bin");
        fs::write(&path, b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq").unwrap();
        let file_hash = sha256_file(&path).unwrap();
        assert_eq!(
            file_hash,
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_expected_hash() {
        let hash = "AB".repeat(32);
        let sums = format!("{hash}  Litty.dmg\n{hash} *litty-macos\nnot-a-hash  litty-linux\n\n");
        assert_eq!(expected_hash(&sums, "litty-macos"), Some("ab".repeat(32)));
        assert_eq!(expected_hash(&sums, "litty-linux"), None);
        assert_eq!(expected_hash(&sums, "litty"), None);
    }

    #[test]
    fn test_verify_signature_rejection_and_cached_check() {
        let dir = temp_dir("sig_test");
        let sums_file = dir.join("SHA256SUMS.txt");
        let minisig_file = dir.join("SHA256SUMS.txt.minisig");
        let gpg_file = dir.join("SHA256SUMS.txt.sig");
        fs::write(&sums_file, b"test content\n").unwrap();
        // Corrupt signature
        fs::write(&minisig_file, b"untrusted comment: signature\nWRONGSIG\n").unwrap();
        fs::write(&gpg_file, b"-----BEGIN PGP SIGNATURE-----\ninvalid\n-----END PGP SIGNATURE-----\n").unwrap();

        let res_mini = verify_signature(&sums_file, &minisig_file);
        assert!(res_mini.is_err(), "tampered or unverifiable minisign signature must be rejected");

        let res_gpg = verify_signature(&sums_file, &gpg_file);
        assert!(res_gpg.is_err(), "tampered or unverifiable GPG signature must be rejected");

        let _ = fs::remove_dir_all(&dir);
    }
}
