# Linux feature validation — 2026-11-04

Status: complete, based on the user's confirmation on 2026-11-04 that all
features have been tested on Linux and run stably.

Linux feature/runtime verification is no longer technical debt. This closes the
previously pending Linux checks for Kitty lock modifiers, Quick Terminal on
X11/Wayland, desktop URL/path opening through `xdg-open`, shell integration
(including fish), and real-app feature flows after the structural refactor.
Linux-specific maintenance proposals in the v0.8.0 backlog are descoped on this
basis, rather than recorded as code fixes.

Evidence source: user-reported testing. Distribution, desktop/compositor,
hardware, tested revision, individual commands and raw logs were not supplied.
No Linux tests were rerun for this documentation update, and no new performance
numbers, provider/version coverage, or automated-suite results are asserted.
Historical reports retain the scope and results of their original runs.

Separate quantitative benchmarks, provider/version coverage, macOS validation,
and unrelated structural follow-ups retain their existing evidence requirements.
