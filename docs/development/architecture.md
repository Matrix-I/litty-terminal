# Architecture

Litty decouples I/O throughput from rendering to guarantee smooth UI interaction even during heavy terminal output:

```text
               ┌──────────────────────────────┐
               │    Shell Process (zsh/bash)  │
               └──────────────┬───────────────┘
                              │ OS PTY Pipe
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Thread 1: PTY Reader Thread                                  │
│  • Reads raw stream via 64 KB static buffer                 │
│  • Parses ANSI / VT sequences via `vte` state machine       │
│  • Writes directly into locked contiguous RingBuffer Grid   │
│  • Dispatches throttled WakeUp event to EventLoop           │
└─────────────────────────────┬───────────────────────────────┘
                              │ Atomic Dirty Flag + WakeUp
                              ▼
┌─────────────────────────────────────────────────────────────┐
│ Thread 2: Main Event & Render Thread (winit + wgpu)          │
│  • Event-driven: Sleeps at 0.0% CPU when idle               │
│  • Synchronized to display V-Sync (120Hz on ProMotion)      │
│  • Maps visible cells into single GPU Instance Buffer       │
│  • 1 Instanced Draw Call via custom WGSL shader             │
└─────────────────────────────────────────────────────────────┘
```

---

Back to the [README](../../README.md).
