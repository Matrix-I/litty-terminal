# macOS App Activation — Partial Implementation Validation

Date: 2026-11-15. Branch: `feat/app-activation`; plan/probe commit `ec69968`,
implementation uncommitted. Base: `af3b5ed`. This report covers process reuse and
pane-selection groundwork. Native notification click-through is incomplete.

## Behavior and Real-App Evidence

`python3 scripts/macos/test-reuse.py target/release/litty` drives the actual
release binary, AppKit windows, PTYs and an isolated shell script. Run this harness only with an empty activation registry and no concurrent
external Litty launches. It refuses existing entries and unsafe/symlinked
registries before dispatch; two safety regressions cover that guard. The script
reports `/bin/pwd -P` and its GUI parent's PID through a private FIFO, then exits
so windows close. It does not modify user dotfiles or send terminal input to an
existing user session. Temporary GUI processes are the harness's own children.

Passed:

- No eligible process: `--reuse` starts a new GUI with its real PTY child at
  the caller's directory.
- A subsequent request opens a window in that same process at a directory
  containing spaces and Vietnamese text.
- An ordinary launch remains a distinct process and keeps ordinary startup
  directory semantics.
- With two eligible processes, the most recently started one receives reuse.
- SIGSTOP of that newest test process makes delivery time out with nonzero CLI
  exit; no shell/window is created in the older available process as fallback.
- After killing/reaping the paused test process, its verified-dead endpoint is
  pruned and a request reaches the remaining process.
- A macOS instance remains eligible after its test shells/windows have closed.

The first harness runs exposed harness-only issues: the system's default
TemporaryDirectory location did not match the test label filter, and Python
3.9 kqueue lacks context-manager support. The harness now uses a private /tmp
fixture and explicit kqueue cleanup; the subsequent complete flow passed.

## Idle Sample

Latest release harness with `--hold`, all its shells exited, one remaining test
instance and no open terminal windows. This sample follows the reviewed cwd and
cleanup fixes:

| Probe | Result |
| --- | --- |
| `footprint -p 22095` | `phys_footprint: 30 MB`; below the closed-window 50 MB budget |
| `ps -p 22095 -o pid=,time=,rss=,state=` | CPU time 0:00.24 → 0:00.25 at least five seconds later; 0.01 CPU s of activity, about 0.2% over a five-second interval |
| RSS | 75,920 → 76,288 KiB; not the physical-footprint budget metric |
| Peak footprint | 53 MB while windows had existed; not the closed-window measurement |

One process/sample, not a statistical performance comparison. The earlier
29 MB sample had flat CPU time; the latest 30 MB sample does not establish the
0% idle CPU target. The observed 0.01 CPU s is not attributed to an unrelated
cause without a baseline. No busy loop was found in the listener. The activation
listener blocks in poll while idle; there is no periodic discovery or expiry
wake-up. Recheck the sample after any later source change affecting the path.

## Regression Evidence

| Area | Before | After |
| --- | --- | --- |
| Framing, path bounds, replay horizon | Four behavioral failures with stubs | Four common tests pass |
| Real Unix activation sockets | Server stub rejects startup | Real kernel socket tests pass outside sandbox |
| Passive activation | Existing AppKit callback emits Reopen, failing exact-focus regression | Passive activation no longer emits generic Reopen; Dock reopen retained |
| Strict cwd / exec | Missing strict API, then child setup checks added | Missing directory/regular file/exec failure reject activation |
| Closed inherited stdio | Subprocess regression reports exec failure as success | Setup and PTY descriptors are normalized above 2; regression also verifies real child echo |
| Interrupted error writes | One-write fixture produces an empty payload | Retried EINTR/short writes preserve all errno bytes |
| Admission after timeout | Admission stub allows another event while one is still queued | A pending reply holds admission until app consumption/drop |
| Crash history | 65 tombstones exhaust the old cap | Only verified-dead owner-private endpoints are removed; registry scan bounded to 4096 entries |
| Pane identity | New selection API absent | Real Tab/PTY test covers split reveal, tab move and stale pane preserving selection |

On this host `UnixStream::set_read_timeout(Some(2s))` returns EINVAL even for a
minimal socket pair outside the sandbox. Production uses nonblocking sockets
and absolute poll deadlines instead, including deadline recomputation after
EINTR. Sandbox Unix socket creation itself returns EPERM; transport/full suites
therefore run outside it.

## Verification Commands

- `cargo check`: passed, zero warnings.
- `cargo check --target x86_64-unknown-linux-gnu`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test` outside sandbox: initial whole-suite run passed 675 unit tests
  plus 1 + 14 integration tests, 12 existing ignored. The final post-review run passed 679 unit + 1 + 14 integration tests
  (694 total), 0 failures, 12 existing ignored.
- `cargo check --target x86_64-apple-darwin`: passed after the reviewed fixes.
- `cargo build --release`: passed; initial binary 2,285,264 bytes. The final post-review rebuild passed; size remains 2,285,264 bytes.
- `git diff --check`: passed after the source fixes; rerun after final docs.
- `python3 -B -m unittest discover -s scripts/macos -p test_reuse_harness.py`: 2 passed.
- `sh scripts/macos/test-activation-probe.sh`: probe compiles/signs with the
  installed SDK; no application shell is executed.
- `python3 -m py_compile scripts/macos/test-reuse.py`: passed outside sandbox;
  sandbox initially denied the normal Python bytecode cache directory.

Additional review corrections: concurrent cleanup treats disappearing sockets
and directories as already removed, while preserving other errors; the internal
monotonic helper is private; the harness refuses populated/unsafe registries.
Their regressions failed before these fixes and pass afterwards. Captured-cwd
tests now cover rename, removal and search-only permissions. On APFS a removed
directory can retain a nonzero link count, so the strict child also checks its
resolved path identity through F_GETPATH/stat before fchdir. Search-only opens
use the SDK's Darwin O_SEARCH value (missing from libc 0.2), with an EINVAL
fallback for older kernels; Linux uses O_PATH. Older macOS runtime remains
unverified. Reference: [Apple fcntl](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fcntl.2.html).

No new test is ignored. Linux runtime and macOS 11–13 runtime are not available
in this session; their affected code is cross-checked, not runtime-verified.

## Self-Review

| Gate | Concrete evidence / limit |
| --- | --- |
| Requirements and edges | CLI tests and real flow cover agreed reuse selection/cwd/fallback behavior. Native notifications remain incomplete and are not marked done. |
| Concurrency | One listener worker; admission survives acknowledgement timeout until the app consumes/drops its event. No grid mutex encloses socket, filesystem, process or PTY setup operations. |
| Error paths | Version/length/UTF-8/path validation, stale sessions, replay cache saturation, rejected acknowledgements and child setup errors have regression coverage. Cross-UID peer rejection is checked in code, not exercised with another user. |
| Performance | No render/parser/scroll hot-path edits or Cell changes. Registry scan is client-side and bounded; worker sleeps without a timer. Latest closed-window sample is 30 MB; its 0.01 CPU s increase means 0% idle CPU is not established by that sample. |
| Security and robustness | Random process sessions/request IDs, getpeereid on both ends, owner-private directories, non-symlink/socket checks, bounded frames/queues/deadlines and no executable IPC operation. Post-fork strict setup uses prepared descriptors, raw errno, write/fchdir/exec/_exit only. |
| Naming / API compatibility | Additive +reuse/--reuse; no existing command/config format migration. activate selector is availability-guarded with macOS 11–13 compatibility. Linux has explicit API twins and ordinary launch stays unchanged. |
| Business assumptions | macOS-only scope, exact pane, opt-in reuse, newest instance, absent-instance startup and timeout error/no fallback were explicitly confirmed. CLI spelling follows existing paired convention. |
| Tests | Boundary, transport, CLI, Tab/PTY and AppKit callback tests plus real GPU/PTY/CLI flow. Notification/native cold-launch and cross-user cases remain unverified. |
| Design | Platform owns OS IPC; app owns window/tab/pane selection; terminal owns strict POSIX child setup. No new layer exceptions, dependencies or keybindings. |

## Native Notification Gate and Review Limits

The separate ad-hoc-signed `Litty Activation Probe` logs normal launch as
`NSApplicationLaunchIsDefaultLaunchKey = 1`. Its authorization request returns
`UNErrorDomain Code=1: Notifications are not allowed for this application`.
No live notification response or cold-notification launch can be characterized
with that permission state. Do not infer modern UN startup behavior from the
legacy NSUserNotification launch key. The maintainer's notification-permission
question is pending; system settings have not been changed.

Thus native notification delivery, origin forwarding across same-bundle
processes, stale-notification cold starts and real notification-driven pane
focus are not implemented or accepted. Existing helper delivery remains intact.

Supplemental interim reviews found and reproduced the low-descriptor,
interrupted-write, admission and tombstone issues; fixes and tests are above.
The strict skills mandate Opus review, unavailable through collaboration here;
no external Claude upload was authorized/attempted. Supplemental review round 1 found the cleanup race and harness isolation
issue above; round 2 uses a fresh package after all fixes. Native completion
remains an open requirement regardless of the partial reuse verdict. The full literal native implementation appendix in
the planning skill also remains unfulfilled because the launch gate is open.

## Supplemental Review Round 2

All three disjoint packages (platform, app/PTY, docs/harness) were reviewed
after the final source fixes. Each reviewer approved the partial reuse scope
with no remaining Critical/Important findings in that scope. The earlier
cleanup race, API visibility, directory mutation coverage and harness safety
findings were verified fixed. No findings were rejected.

This is two supplemental whole-change review rounds plus earlier interim
feedback, not the required Opus verdict. The native notification requirement
remains unmet. Reviewers did not rerun supplied tests. They explicitly leave
older macOS/Linux runtime, cross-user checks, native notification lifecycle
and 0% idle CPU unverified. On old macOS without O_SEARCH, the fallback supports
readable directories only; search-only directory support is verified on this
host, not on that fallback.

## Notification Permission Follow-up

The maintainer explicitly authorized enabling notifications for the local probe.
The attempt to open System Settings through computer use failed with
`Computer Use permissions are not granted`. No setting was changed. This is
a separate tool-access blocker from the probe's notification authorization
error; the earlier request for permission is now answered. Native notification
characterization remains incomplete until computer control is enabled or the
maintainer enables the probe's notification setting manually.

## Probe Registration Follow-up

The maintainer could not find Litty/probe in the notification settings list.
This does not establish a user-denied setting. `codesign --verify --strict`
passes on the ad-hoc test bundle; its signature identifier is
`com.litty.activation-probe` and its Info.plist is sealed. Explicit
LaunchServices registration and relaunch from /tmp reproduce UNErrorDomain=1.
A copy of the same signed bundle launched from the ignored project build
directory `dist/activation-test/` reaches didFinish/becameActive and has not
yet returned its notification authorization callback. A permission-dialog
check is pending; location as the root cause is not established yet.
Narrowly filtered usernoted logs are unavailable: the OS reports Operation
not permitted even for the approved outside-sandbox diagnostic. No notification
database/reset, code-signing identity, entitlement or system preference was
modified.

## Notification Authorization Confirmed

The maintainer selected Allow for the project-build-directory probe. Log for
process 26938 confirms `permission 1 (null)` and `delivery (null)`: authorization
and submission succeeded. This supersedes the earlier authorization blocker
for this registered bundle. The /tmp launches still reproduced denial; no
claim is made that every ad-hoc build/location behaves identically. Live-click
callback and cold-notification launch ordering are the next experiment gates.
No production source changes were needed for this authorization observation.

## Live Notification Response Observed

Probe process 26938 logged `response com.apple.UNNotificationDefaultActionIdentifier`
followed by `becameActive` and `reopen`. This confirms live default-action
delivery and shows a generic reopen can follow a notification response; future
integration must preserve the specific target across that ordering. A further
notification was delivered by process 31989, then the three verified task-owned
probe processes were stopped with SIGTERM. A cold click is pending.

The probe bundle deliberately has no icon resources/CFBundleIconFile, so its
notification icon is not evidence for Litty's packaged icon behavior. The real
Info.plist/icon packaging must be checked in final native acceptance.

## Cold Notification Launch Observed

Following the maintainer's click after origin exit, process 34052 logged main,
willFinish, then didFinish with default=0 and
NSApplicationLaunchUserNotificationKey=UNNotificationResponse, then default
response and becameActive. No reopen was logged for this cold launch. This
resolves the startup-characterization gate on this host and supersedes earlier
blocked notes. Native source integration is now in progress and is not yet
verified; earlier 694-test results apply only to the preceding reuse source.

## Native Integration Build Checkpoint

The native bridge and app routing are now implemented but real desktop
acceptance/review remains in progress. Early delegate setup precedes
finishLaunching; app startup policy suppresses the initial PTY for a
notification-only launch. Callback work forwards through a four-entry worker
queue to the exact origin session. Linux retains helper delivery. A test
Littty bundle includes the real release binary and Litty.icns under the
already-authorized probe identity; visual icon/focus confirmation is pending.

Latest initial native full suite: 683 unit + 1 + 14 integration tests passed
(698 total), 12 existing ignored. Native callback tests were added alongside
the implementation, so no initial red was recorded for that portion; this is
a test-first process gap. Cold startup policy test did fail before its fix.
Apple x86_64 and Linux checks, Clippy with denied warnings and release build
passed. Re-run final gates if subsequent review changes source.

## Native Review Corrections (in progress)

The reviewer confirmed authorization timeout included human decision time and
clean-exit withdrawal was missing. Native regressions failed before the fixes.
Submission deadlines now start after authorization; pending admission remains
bounded to eight while waiting for the user. Session-owned notification IDs
are tracked up to 64; shutdown removes only those pending/delivered IDs, clears
its delegate if still owned, and removes its launch observer. Late completion
blocks withdraw their exact ID after shutdown or retention eviction. Snapshot
cleanup was also corrected to copy IDs before clearing the ordered set.

Notification-associated reopen suppression still has a plausible ordering race:
a queued main-thread clear may precede the OS reopen. This remains an Important
open finding until real ordering tests or causal event handling prove it safe.
Do not claim native integration complete. The real Litty test bundle is running
with its icon; the maintainer's visual icon/focus confirmation is pending.

## Checkpoint Verification

After the native authorization/cleanup fixes: full unsandboxed cargo test passes
685 unit + 1 + 14 integration tests (700 total), with 12 existing ignored.
macOS/Linux checks, Apple x86_64 check and Clippy pass. The latest source still
has the open reopen-ordering finding and unconfirmed real bundle icon/focus;
checkpoint commit was explicitly requested despite those recorded limitations.
