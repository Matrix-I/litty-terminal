//! Cancellable curl transport. Secrets and bodies go through stdin, never argv.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const MAX_RESPONSE: u64 = 8 * 1024 * 1024;
pub struct Reply {
    pub status: u16,
    pub body: String,
}
fn quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
fn cancelled(flag: &AtomicBool) -> Result<(), String> {
    if flag.load(Ordering::Acquire) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
fn wait(flag: &AtomicBool, duration: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < duration {
        cancelled(flag)?;
        std::thread::sleep(Duration::from_millis(20).min(duration.saturating_sub(start.elapsed())));
    }
    cancelled(flag)
}
fn once(
    url: &str,
    headers: &[String],
    body: Option<&str>,
    cancel: &Arc<AtomicBool>,
) -> Result<Reply, String> {
    cancelled(cancel)?;
    let mut config = format!("url = {}\n", quote(url));
    for h in headers {
        config.push_str(&format!("header = {}\n", quote(h)));
    }
    if let Some(b) = body {
        config.push_str("request = \"POST\"\n");
        // data-raw: data-binary treats a leading '@' as a local file to upload (CWE-73).
        config.push_str(&format!("data-raw = {}\n", quote(b)));
    }
    let bin = if std::path::Path::new("/usr/bin/curl").exists() {
        "/usr/bin/curl"
    } else {
        "curl"
    };
    // No redirects: credentials must never follow a redirect to another origin.
    let mut child = Command::new(bin)
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            "180",
            "--max-filesize",
            "8388608",
            "--proto",
            "=http,https",
            "--write-out",
            "\n%{http_code}",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Cannot start curl".to_string())?;
    let mut stdin = child.stdin.take().ok_or("curl stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("curl stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("curl stderr unavailable")?;
    // Drain both pipes concurrently: large requests/responses must not deadlock.
    let writer = std::thread::spawn(move || stdin.write_all(config.as_bytes()));
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        stdout
            .take(MAX_RESPONSE + 16)
            .read_to_end(&mut b)
            .map(|_| b)
    });
    let errors = std::thread::spawn(move || {
        let mut b = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut b).map(|_| b)
    });
    let status = loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            let _ = reader.join();
            let _ = errors.join();
            return Err("Cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = reader.join();
                let _ = errors.join();
                return Err("Cannot wait for curl".into());
            }
        }
    };
    let written = writer.join().map_err(|_| "curl writer failed")?;
    let bytes = reader
        .join()
        .map_err(|_| "curl reader failed")?
        .map_err(|_| "Cannot read curl response")?;
    let _ = errors.join();
    cancelled(cancel)?;
    if !status.success() || written.is_err() {
        return Err(format!(
            "AI transport failed (curl exit {:?})",
            status.code()
        ));
    }
    if bytes.len() as u64 > MAX_RESPONSE {
        return Err("AI response exceeds 8 MiB limit".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "AI response is not UTF-8")?;
    let (body, code) = text.rsplit_once('\n').ok_or("Missing HTTP status")?;
    Ok(Reply {
        status: code.parse().map_err(|_| "Invalid HTTP status")?,
        body: body.into(),
    })
}
pub fn request(
    url: &str,
    headers: &[String],
    body: Option<&str>,
    cancel: &Arc<AtomicBool>,
    status: &impl Fn(String),
    last: &mut String,
) -> Result<String, String> {
    for attempt in 0..10 {
        cancelled(cancel)?;
        let result = once(url, headers, body, cancel);
        let error = match result {
            Ok(reply) => {
                *last = reply.body.clone();
                if (200..300).contains(&reply.status) {
                    return Ok(reply.body);
                }
                let e = format!(
                    "AI HTTP {}. Use yo show last response for details.",
                    reply.status
                );
                if !retryable(reply.status) {
                    return Err(e);
                }
                e
            }
            Err(e) if e == "Cancelled" => return Err(e),
            Err(e) => e,
        };
        if attempt == 9 {
            return Err(error);
        }
        status(format!("[attempt {}/10] Waiting...", attempt + 2));
        wait(cancel, Duration::from_secs((1u64 << attempt).min(60)))?;
        status(format!("[attempt {}/10] Thinking...", attempt + 2));
    }
    unreachable!()
}
fn retryable(status: u16) -> bool {
    matches!(status, 408 | 429 | 500..=599)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_policy_and_cancelled_backoff() {
        for code in [408, 429, 500, 503, 599] {
            assert!(retryable(code));
        }
        for code in [400, 401, 403, 404, 422] {
            assert!(!retryable(code));
        }
        assert!(wait(&AtomicBool::new(true), Duration::from_secs(60)).is_err());
    }
    #[test]
    fn curl_config_escapes() {
        assert_eq!(quote("a\"\\\nb"), "\"a\\\"\\\\\\nb\"");
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn cancels_an_inflight_curl_process() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let _stream = loop {
                if let Ok((s, _)) = listener.accept() {
                    break s;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            };
            std::thread::sleep(Duration::from_millis(50));
            flag.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_millis(100));
        });
        let start = Instant::now();
        let result = once(&url, &[], Some("{}"), &cancel);
        assert_eq!(result.err().as_deref(), Some("Cancelled"));
        assert!(start.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;

    /// Serves one request and returns the raw request body it received.
    fn capture_body(payload: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= len || n == 0 {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&raw)
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default()
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let reply = once(&url, &[], Some(payload), &cancel).unwrap();
        assert_eq!(reply.status, 200);
        server.join().unwrap()
    }

    #[test]
    fn payload_starting_with_at_is_sent_literally() {
        // CWE-73: curl's data-binary would read /etc/hosts from disk.
        assert_eq!(capture_body("@/etc/hosts"), "@/etc/hosts");
    }

    #[test]
    fn plain_payload_is_sent_verbatim() {
        assert_eq!(capture_body("{\"a\":\"b\\n\"}"), "{\"a\":\"b\\n\"}");
    }
}
