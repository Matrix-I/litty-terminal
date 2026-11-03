//! End-to-end: `litty +bench-dsr` drives a real PTY through the production reader.

use std::process::Command;

#[test]
fn dsr_benchmark_completes_on_a_real_pty() {
    let out = Command::new(env!("CARGO_BIN_EXE_litty"))
        .args(["+bench-dsr", "50"])
        .output()
        .expect("run litty +bench-dsr");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Latency targets are reported, not asserted: they depend on the host.
    assert!(stdout.contains("samples: 50"), "stdout: {stdout}\nstderr: {stderr}");
    assert!(!stderr.contains("failed"), "stderr: {stderr}");
}
