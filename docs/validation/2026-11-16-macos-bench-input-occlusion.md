# macOS `+bench-input-latency` covered-window check — 2026-11-16

## Verdict

A real covered window now fails the benchmark promptly with the intended
message instead of timing out with "no frame showed the echo". This closes
the manual check left open by the v0.14.0 plan. No production code changed.

## Environment and method

- Source: `4437e4e` (unmodified), `cargo build --release`.
- Host: macOS 27.2.1, build 26B131, Darwin 27.2.0, arm64.
- Occluder: a throwaway Swift/AppKit helper (not in the repository) that
  shows an opaque borderless black window at `.screenSaver` level over every
  screen for a set time. It is an ordinary window of another app, so the
  benchmark window receives a real `WindowEvent::Occluded(true)` from AppKit;
  no Accessibility permission or app-hiding API is involved.
- Each covered run: `litty +bench-input-latency 20000` (long enough that it
  cannot finish normally), with the occluder started 3 s after launch, after
  the echo child is ready and measuring has begun.

## Observations

| Run | Window | Result | Exit after occluder start |
| --- | --- | --- | ---: |
| Baseline, 300 samples | visible | report: p50 4.10 ms, p95 10.76 ms, p99 14.87 ms | — |
| Covered 1 (8 s cover) | covered | `benchmark failed: the benchmark window was covered or hidden while measuring; …`, exit 1 | 13.6 s total run, see note |
| Covered 2 (20 s cover) | covered | same message, exit 1 | 1.19 s |
| Covered 3–5 (20 s cover) | covered | same message, exit 1 | 1.24 s, 1.21 s, 1.20 s |

The exit delays include launching the helper process and AppKit
delivering the occlusion change, and stay below the 2 s
`SAMPLE_DEADLINE`, so the failure comes from the occlusion check, not from
the sample timeout (whose message differs). None of the covered runs
printed "no frame showed the echo".

Note on covered run 1: it was the first launch of the freshly compiled
helper and its exit timing was not recorded against the helper's start, so
it only shows the message, not the delay.

## Not verified

- Occlusion by moving the window to another Space, minimizing, or hiding
  the app (Cmd-H); these go through the same `Occluded` event but were not
  exercised here.
- The benchmark's figures were not re-measured on an idle machine; the
  baseline row is a smoke check, not a performance.md refresh.
