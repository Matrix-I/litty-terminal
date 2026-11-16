# macOS APC buffer retention investigation — 2026-11-16

## Verdict

Keep the existing bounded APC buffer reuse policy. This investigation closes
v0.14.0's optional retention check without a production-code change. It does
not establish an app latency improvement or a physical-footprint reduction.

## Environment and method

- Source: `918aeaf90ab7db7b792e43eaab7c4e6040e21f5f` (unmodified production code).
- Host: macOS 27.2.1, build 26B131, Darwin 27.2.0, arm64.
- Native Rust debug tests using the binary's macOS mimalloc allocator.
- An isolated `git archive HEAD` tree at `/tmp/litty-apc-retention-918aeaf`
  received only the diagnostic test reproduced below. No repository source
  or dependencies changed.
- Real `Grid::feed_byte` APC parsing and Kitty assembly/decoding: 100 RGB
  images, 64 × 64 pixels, four 4,096-byte base64 payloads per image. Each
  completion checks dimensions, decoded length, first pixel and OK reply.
- Comparison: retain capacity normally versus explicitly discard `apc_buf`
  after every dispatched chunk in the test. This models the proposed release
  policy; it is not a patched production implementation.

## Observations

| Policy | Chunks requiring capacity growth | Chunks reusing the same pointer | Final APC capacity |
| --- | ---: | ---: | ---: |
| Existing retention | 1 | 399 | 8,192 bytes |
| Discard after every chunk | 400 | 0 | 0 bytes |

These are capacity/pointer observations, not counts of every allocator call.
Other image buffers and decoding allocations are outside this comparison.

The probe also confirms:

- Completed `m=0` leaves capacity available for the next transfer.
- Ordinary terminal output and alternate-screen entry/exit retain capacity;
  it is one buffer per grid, not one retained buffer per image or chunk.
- Full reset and clear-screen-and-history release capacity to zero.
- Dispatch retains a capacity of exactly 65,536 bytes and releases a capacity
  above that threshold. A real completed APC exceeding 65,536 bytes also
  releases it; the parser accepts a subsequent Kitty chunk.
- CAN cancellation releases the new buffer.

Code rationale: `apc_dispatch` takes and restores the same `Vec`; the next
`ESC _` clears its length before reuse. Reset and screen clearing already
clear and shrink it. `set_alt` does not change this buffer. The retained body
bytes are not securely erased; this investigation makes no secure-erasure claim.
The allocation remains bounded at 64 KiB per grid after dispatch, even at idle.
Freeing it at every `m=0` or alternate-screen exit would trade that bounded
retention for capacity growth on the next transfer; no demonstrated need for
that trade was found in this workload.

## Verification and limits

Passed outside sandbox:

```sh
cargo test --manifest-path /tmp/litty-apc-retention-918aeaf/Cargo.toml --target-dir /Users/linh.nguyen/Desktop/personal/litty-terminal/target --bin litty terminal::grid:: -- --nocapture
```

Result: **154 passed, 0 failed, 6 existing ignored**, including the probe.
Log: `/tmp/litty-apc-retention-grid.log` (temporary local artifact).

The initial sandbox command omitted `--bin litty`. The diagnostic test passed,
but the command exited 101 because the separate `native_activation` executable
could not obtain a Metal GPU (`MTLCreateSystemDefaultDevice failed: no Metal
GPU`). On the maintainer's request, the identical filtered command was rerun
outside sandbox without `--bin litty`:

```sh
cargo test --manifest-path /tmp/litty-apc-retention-918aeaf/Cargo.toml --target-dir /Users/linh.nguyen/Desktop/personal/litty-terminal/target apc_retention_investigation_native -- --nocapture
```

Exit 0: the probe passed with the same 1/399 versus 400/0 growth/reuse results,
and the native AppKit runner passed (cold-start window/PTY, stale pane,
menu/service, hidden-tab activation, missing-directory rejection, benchmark
exclusions and shutdown cleanup). The Metal GPU error did not recur outside
sandbox. Log: `/tmp/litty-apc-retention-unsandboxed.log`.

The full suite was then run outside sandbox on the same snapshot plus probe:

```sh
cargo test --manifest-path /tmp/litty-apc-retention-918aeaf/Cargo.toml --target-dir /Users/linh.nguyen/Desktop/personal/litty-terminal/target
```

Exit 0: **710 unit tests passed, 12 existing ignored**, real-PTY DSR integration
**1 passed**, shell integrations **14 passed**, and the native AppKit runner
**PASS**. Log: `/tmp/litty-apc-retention-full-unsandboxed.log`. The suite printed
`ditto: Couldn't read PKZip signature` during its passing bundle-install test;
no test failed.

No GUI/PTY end-to-end image transfer, release timing benchmark, RSS/footprint
measurement, Linux runtime test, separate build or Clippy run was
performed. The probe exercises actual parser/assembly/decoding in-process,
not image rendering. Twelve existing ignored unit tests (including six grid
tests) remain unverified. The AppKit runner exercises real windows and PTYs,
but does not render the APC probe image.
No failing regression or fix is claimed: retention was an optimization
candidate, and reuse is observed. Implementation-plan TDD/fix/review steps do
not apply to this investigation-only documentation change.

## Reproducible diagnostic test

Append this test to `src/terminal/grid/tests.rs` in an isolated source snapshot
of the commit above, then run the command with that snapshot's manifest path.
The explicit buffer discard is confined to the comparison test.

```rust
#[test]
fn apc_retention_investigation_native() {
    let payload = "A".repeat(4096);
    let first = format!("\x1b_Ga=t,f=24,s=64,v=64,i=1,m=1;{payload}\x1b\\");
    let middle = format!("\x1b_Gm=1;{payload}\x1b\\");
    let last = format!("\x1b_Gm=0;{payload}\x1b\\");
    for release_each in [false, true] {
        let mut g = Grid::new(20, 3);
        let mut growth_chunks = 0;
        let mut reused_chunks = 0;
        for _ in 0..100 {
            for chunk in [&first, &middle, &middle, &last] {
                let before = g.apc_buf.capacity();
                let ptr = g.apc_buf.as_ptr();
                feed(&mut g, chunk);
                assert_eq!(g.apc_state, 0);
                assert!(g.apc_start.is_none());
                assert!(g.apc_buf.capacity() <= 65536);
                if g.apc_buf.capacity() > before { growth_chunks += 1; }
                else {
                    assert_eq!(ptr, g.apc_buf.as_ptr());
                    reused_chunks += 1;
                }
                if release_each {
                    g.apc_buf = Vec::new();
                }
            }
            let img = g.graphics.image(1).expect("completed transfer");
            assert_eq!((img.width, img.height), (64, 64));
            assert_eq!(&img.rgba[..4], &[0, 0, 0, 255]);
            assert_eq!(img.rgba.len(), 64 * 64 * 4);
            assert!(g.response.ends_with(b"i=1;OK\x1b\\"));
            g.response.clear();
        }
        eprintln!("release_each={release_each} growth_chunks={growth_chunks} reused_chunks={reused_chunks} retained_capacity={}", g.apc_buf.capacity());
        assert_eq!(growth_chunks, if release_each {400} else {1});
        assert_eq!(reused_chunks, if release_each {0} else {399});
        if !release_each {
            let capacity = g.apc_buf.capacity();
            let ptr = g.apc_buf.as_ptr();
            feed(&mut g, "ordinary terminal output");
            assert_eq!(g.apc_buf.capacity(), capacity);
            g.set_alt(true, false);
            g.set_alt(false, false);
            assert_eq!(g.apc_buf.capacity(), capacity);
            assert_eq!(g.apc_buf.as_ptr(), ptr);
            g.full_reset();
            assert_eq!(g.apc_buf.capacity(), 0);
            feed(&mut g, &first);
            g.clear_screen_and_history();
            assert_eq!(g.apc_buf.capacity(), 0);
        }
    }
    // Check the dispatch retention threshold independently of allocator growth.
    for capacity in [65536, 65537] {
        let mut g = Grid::new(20, 3);
        g.apc_buf = Vec::with_capacity(capacity);
        let allocated = g.apc_buf.capacity();
        g.apc_buf.extend_from_slice(b"not-kitty");
        g.apc_dispatch();
        assert_eq!(g.apc_buf.capacity(), if allocated > 65536 {0} else {allocated});
    }
    // Exercise actual parser eviction of a large completed APC and recovery.
    let mut g = Grid::new(20, 3);
    feed(&mut g, &format!("\x1b_{}\x1b\\", "x".repeat(65537)));
    assert_eq!(g.apc_buf.capacity(), 0);
    feed(&mut g, &first);
    assert!(g.apc_buf.capacity() > 0);
    feed(&mut g, "\x1b_unfinished\x18");
    assert_eq!(g.apc_buf.capacity(), 0);
}
```
