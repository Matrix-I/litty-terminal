# macOS App Activation — Partial Implementation Validation

Date: 2026-11-15. Branch: `feat/app-activation`; base: `af3b5ed`.
Implementation checkpoints `8df30d5` and `8fd884e` are committed locally;
the lifecycle regression and later documentation remain uncommitted.
Native notification delivery/routing/startup handling is implemented, but full
desktop acceptance remains incomplete. Later sections supersede earlier
permission blockers and intermediate test counts; they preserve the execution
record rather than asserting those blockers still apply.

Current verification: 701 ordinary tests plus the real AppKit lifecycle runner
pass, 12 existing ignored; macOS/Linux checks, Clippy and release build pass.
Supplemental review approves the lifecycle batch. Physical icon/focus,
stale-click and Dock acceptance, older macOS runtime, exact 0% idle CPU and
required Opus review remain unverified. The guide and ROADMAP now reflect this
state without claiming completion.

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

## Reopen Source Fix

Checkpoint `8df30d5` was committed at the maintainer's request. Work continued
with a focused same-identity probe. Notification click logged aevt/rapp whose
read-only sender PID resolved to /usr/sbin/usernoted; ordinary open logged a
different sender. Production no longer uses a flag cleared on a main-queue
turn. It inspects the current reopen event synchronously and suppresses only
the system notification daemon source. The regression failed before this
classifier, then all five native tests passed.

After a notification-only cold launch, a later ordinary reopen now starts the
activation service after successfully opening its first window. The existing
startup service block was extracted for that lifecycle path. Real cold-then-Dock
GUI acceptance for this additional path remains pending. The earlier full
reopen-source suite passed 686 unit + 1 + 14 integration tests (701 total),
12 ignored; final rerun follows the service-lifecycle adjustment.

This eliminates the reviewed queue-timing dependency on this observed macOS
source. Apple does not promise every supported OS version uses usernoted for
this event; older macOS runtime remains unverified. Unknown event sources
retain ordinary reopen behavior. Visual app icon/focus and actual Dock-source
comparison remain pending maintainer confirmation, not claimed as passed.

## Second Requested Checkpoint

Final source checks before this checkpoint: `cargo check`,
`cargo check --target x86_64-unknown-linux-gnu`,
`cargo clippy --all-targets -- -D warnings`, and `git diff --check` pass.
Unsandboxed `cargo test` passes 686 unit + 1 + 14 integration tests (701 total),
with 12 existing ignored tests. No keybindings or public formats changed.
Full diff and untracked-file inspection found only the five intended paths.

Self-review: activation starts only after a successful window creation, once
per live server, and remains excluded from all benchmark modes. Event-source
inspection is synchronous, bounded and performs no grid locking or subprocess
launch. Unknown senders preserve ordinary reopen behavior. Native tests cover
source classification and callback completion, not physical desktop clicks.
The supplemental reviewer confirmed the lifecycle fix but requested an
observable cold-start/menu/service regression; that Important test gap remains
open for the next batch. Required Opus review remains unavailable in this
runtime; supplemental review does not satisfy that model requirement.
This requested checkpoint is incomplete feature work, not final acceptance.

## Observable Lifecycle Regression

Checkpoint `8fd884e` is committed locally. The following test-only batch remains
uncommitted. `cargo test --test native_activation` runs a dedicated executable
on main so AppKit can create real windows. It injects only the OS notification
startup classification, then dispatches the actual platform NewWindow menu
event through App::user_event. Assertions cover: no initial window/service,
one menu-created window, a live /bin/cat PTY child, this process's socket file,
stable service identity, all five benchmark exclusions, and socket removal
after shutdown. No synthetic UI input or global reuse discovery is used.

Mutation evidence: temporarily removing start_activation_service from the
common open_window path produces `menu window did not start activation
service` in `/tmp/litty-native-lifecycle-red.log`; restoring it passes. The
watchdog initially mislabeled a disconnected completion channel as a deadline;
it now distinguishes timeout from disconnection. The first fixture link also
failed because mi_collect lacked mimalloc linkage; the test executable now
explicitly links the existing dependency. No dependency was added or upgraded.

Final commands after the last source edit:

- `cargo check`: pass.
- `cargo check --target x86_64-unknown-linux-gnu --all-targets`: pass, including
  the new target; Linux explicitly reports the AppKit test is unavailable.
- `cargo test` outside the sandbox: 686 unit + 1 + 14 integration tests pass,
  plus the native lifecycle executable PASS; 12 existing ignored unchanged.
  Log: `/tmp/litty-native-lifecycle-final.log`.
- `cargo clippy --all-targets -- -D warnings`: pass after fixing fixture
  reachability warnings. The separate fixture allows dead code/unused imports
  from compiling the binary's modules without its CLI, and
  items_after_test_module for inherited source ordering. Normal crate linting
  retains its original policy; this scoped allowance does not skip any tests.
- `cargo build --release`: pass.
- `git diff --check`: pass.

Self-review: only test startup classification is overridden, per-thread and
absent from release builds. Real window/menu/service code is exercised; the
fixture never launches the user's shell or sends requests to another process.
The watchdog bounds native initialization; normal shutdown drops PTYs/server
and verifies socket removal. No hot paths, locks, formats or dependencies
change. The binary-only module list is duplicated in the fixture to avoid a
production library/API refactor. Physical desktop clicks and older OS behavior
remain outside this test's evidence. Required Opus and receiving-code-review
sub-skill are unavailable locally; supplemental review remains supplementary.

The refreshed real Litty bundle (authorized probe identity, Litty.icns, latest
release binary) is running as PID 79480 with an empty stderr log. Visual
icon/focus confirmation is pending, not counted as acceptance.

Supplemental review of the final-source package MPoOnJ: Approved, one round,
no findings. The reviewer traced the actual menu/window/service wiring,
benchmark exclusions, isolated per-thread startup override, owned endpoint
cleanup and scoped fixture lint policy. The previous Important coverage gap
is resolved; none were rejected. Not verified by that reviewer: supplied logs
were not rerun, native launch classification is injected, and physical desktop
clicks/older OS behavior/required Opus remain outside its evidence. Only these
review-status documentation paragraphs changed after source verification.

## Native Idle Sample and Current Desktop Blocker

A separate disposable instance of the latest native test bundle ran with
SHELL=/usr/bin/true. Its PTY exited, no child processes remained, and its own
activation socket existed. Closing the last pane follows App::on_child_exit;
this sample did not directly observe an OS window count. Instance PID 80735
was terminated and only its owned endpoint removed; the interactive
notification test was preserved. Raw results: `/tmp/litty-native-idle.json`.

- `footprint -p 80735`: phys_footprint 24 MB, peak 24 MB, exit 0.
- Four ps samples over 30.10 seconds: 0:00.22, 0:00.23, 0:00.23, 0:00.24
  CPU time; each reports 0.0% CPU. The 0.02 CPU s increase is approximately
  0.07% over the measured interval; rounded ps output does not prove exact zero.
- Native test stderr: empty.

The documentation-only package i0MvB2 was supplementally approved without
findings. No source changed after its reviewed 701-test/lifecycle verification.
This measurement section was added afterwards and is not independently reviewed.

Desktop access was revalidated: cua.getState returned an app inventory, but
cua.getApp for the actual test bundle returned `Computer Use permissions are
not granted`. Inventory success did not establish UI access. No mouse/keyboard
action occurred, no settings changed, and no alternate UI automation was used.
The earlier visual-result question is still unanswered. Actual notification
icon/focus, stale-click and Dock verification therefore require user input or
working computer-use access. Opus/older macOS requirements still need a
suitable external environment; feature completion is not claimed.

## Maintainer Icon Check

The maintainer first reported the test notification's logo was incorrect,
then confirmed `đã thấy dùng đúng logo` (the correct logo is now visible).
The visual logo check is passed by maintainer observation. Focus, stale clicks,
cold-start no-shell behavior and Dock checks have not been confirmed by this
message. The notification title/sender identity was not specified, so this
does not separately establish both bundle identities' acceptance.

Diagnostics before that confirmation: the probe bundle's Litty.icns hash
matches resources/Litty.icns; its plist names that resource, the signature
seals it, and NSWorkspace resolves the probe ID to that bundle. Extracted
small icon representations show the expected Litty logo. Several older
iconless probes share com.litty.activation-probe, but a causal cache defect
was not proven. No production code or icon resources were changed.

A separate isolated bundle using the real com.litty.terminal identity was
packaged from the latest release binary and exact resources/Info.plist/icon,
signed and verified. Its fixed shell ran (PID 84496), but native stderr reports
`notification authorization unavailable`; delivery from that identity was
not established. It did not replace or terminate the installed user app.
Do not attribute the maintainer's visual success to a code fix or assume this
separate permission gate was resolved. Runtime test artifacts stay in dist.

## Maintainer Live-Click Check

The fresh authorized-probe test used the actual release Litty executable,
window title `Litty native notification test`, and notification title
`Litty activation test`. Process 84899 had a live /bin/cat PTY child 84903
and empty native stderr before the click. The maintainer confirmed
`đúng k tạo thêm cửa sổ` (correct, no additional window).
Record the live single-pane focus/no-extra-window check as passed. This
does not establish moved-tab, hidden-split, stale-pane or cold-launch behavior.

Next manual fixture: `Litty stale-pane test`, process 85098. The maintainer
is to close only that named test window before clicking its notification.
No stale-pane result has been received yet.

Stale-pane fixture follow-up: the PTY was still live when the maintainer asked
to continue. After verifying PID 85101 was exactly /bin/cat with parent 85098,
the agent sent SIGTERM only to that disposable child. A subsequent ps showed
the app process 85098 alive and child 85101 gone; native stderr remains empty.
This exercises the real child-exit pane-close path without UI input. The
notification click/result is still pending and is not recorded as passed.

Correction to the earlier informal cold-launch checklist: clean Cmd+Q calls
litty_notifications_shutdown and withdraws only this process's notifications.
Thus a clean quit may leave no notification to click. Testing a retained
notification after a crash needs a separate owned-process crash fixture;
do not require a clean-quit notification to remain visible or mark that path
passed based only on the live or closed-pane check.

## Reported Stale-Click Regression and Diagnostic Replay

The maintainer reported that clicking with no windows opened one window and
appeared hung, supplying a screenshot of `Litty stale-pane test` echoing input.
The fixture runs /bin/cat, which echoes rather than executing shell commands;
that explains its shell-like appearance but does not excuse an unwanted window.
Treat the extra-window report as an open regression, not a passed stale check.
The original test processes were no longer running when inspected, so their
actual reopen provenance could not be recovered. Preserve the report without
inventing a cause or claiming a fix.

A bounded temporary diagnostic library under /tmp/litty-stale-click-diagnostic
records native delivery/response and the current Apple event's sender metadata,
without terminal text. It delegates to the original methods; no production
source was edited. It was injected only into disposable process 86325 using
the same test bundle (Mach-O UUID 1AE6BF6F-A13F-3CED-8D45-6B7AA38A739A).
The diagnostic confirmed native delivery before the fixed /bin/cat child 86326
was terminated. The maintainer then clicked `Litty stale-repro` and reported
no window opened. The trace recorded aevt/rapp, sender PID 88005 resolving to
/usr/sbin/usernoted, matching the existing suppression classifier.

This replay passes but does not explain the earlier report; tracing can affect
timing. A second `Litty stale-final` fixture (process 86653) runs without any
diagnostic library. Its manual full-window-close then notification-click result
is pending. No speculative production fix or new regression assertion has been
added; bugfix-strict's reproduction/root-cause gate remains unresolved.

## Uninstrumented Closed-Pane Confirmation

The resent baseline fixture `Litty stale check #2` ran as process 87795,
without DYLD_INSERT_LIBRARIES. The maintainer closed its window and clicked
the notification. The app remained alive with no PTY child, and native stderr
was empty. Asked whether UI also showed no replacement window or wrong-pane
selection, the maintainer answered `yes`. Record this uninstrumented stale-pane
case as passed. The earlier extra-window report remains an unexplained
intermittent report; this confirmation is not evidence of a production fix.

The next distinct fixture is `Litty cold-start check`. Native delivery will be
confirmed before crashing only the originating disposable process and stopping
the previous empty probe instance. A subsequent click must then cold-launch
without creating a shell. No cold-start result is inferred from the earlier
`clicked` message, which arrived before this new notification was prepared.

## Cold-Start Requirement Amendment

After clicking `Litty cold-start check`, the maintainer explicitly stated that
cold-start should open a new window, as expected. Treat this as a requirement
amendment: a fully exited app may create one fresh normal window when an old
notification launches it. A stale pane in a still-running app remains ignored.
Do not mark fresh-window cold-start behavior as a defect under the new policy.

Before the click, native delivery was confirmed, originating process 88727
was killed, and the only previous empty test process 87795 was stopped. No
test-bundle process remained. After the click, process 88736 appeared; a later
snapshot found no child shell but one activation endpoint. The maintainer's
window observation and this later process snapshot cover different moments;
neither should be replaced with an invented chronology.

The source still has opens_initial_window(true, false, false) returning false,
with a unit test asserting the old no-shell policy. That policy and its
lifecycle fixture must be aligned with the amended requirement before
completion is claimed. No source was changed in recording this amendment.

## Hidden-Tab Selection and Foreground Check

Uninstrumented `Litty hidden-pane #2` used process 90035 with source A and
another tab B. The maintainer reported that clicking selected SOURCE_A but
left the window behind other UI. This is an open foreground-activation report;
correct tab selection alone does not pass the full check. Another disposable
same-bundle process 89799 was also alive. The receiver process and activation
state of that failing click were not captured, so a cross-process handoff
problem is a hypothesis, not a proven cause.

For a diagnostic replay, only those two old disposable processes were stopped.
Process 90725 ran the same release test bundle with a temporary bounded trace
of native response routing, window-front calls, activation requests and its
own active/hidden state. The maintainer reported `ok, now very good`.
The trace shows the native response and origin both in process 90725, followed
by the activation request and NSApplicationDidBecomeActive with active=1,
hidden=0, keyWindow present. The user-facing hidden-tab/foreground replay
passes in this isolated diagnostic case. No production source was changed.

Log: /tmp/litty-focus-diagnostic/events.log. It contains lifecycle/target-PID
metadata, not terminal contents. Immediate active=0 after activate is followed
by a later became-active event; an immediate query alone would misdiagnose
this asynchronous API. The prior multi-instance foreground failure remains
unexplained until the separate cross-process test is traced. Zoomed split
reveal is not covered by this tab-only check. Reference: Apple's cooperative
activation documentation requires yielding before cross-app activation and
does not guarantee every activation request is granted.

## Two-Process Foreground Check

The controlled fixture /tmp/litty-two-process-vuf40uey launched source A as
PID 91228 and confirmed native delivery before launching newer B as PID 91235.
Both had distinct live /bin/cat PTYs and empty stderr. The maintainer worked
from B, clicked `Litty 2-process: return A`, and reported `ok very good`.
Record this two-live-process UI check as passed: the newer process did not
steal the notification target, and A came forward.

The trace in /tmp/litty-focus-diagnostic/events.log shows B active before
the click, native-response in A with response-target=91228, then B resigning
and A becoming active with a key window. macOS delivered the callback directly
to its origin in this sample. Thus it does not establish the separate case
where B receives A's callback and forwards it across IPC. The older foreground
failure is still not causally explained; no production fix was made. Both
temporary instances were limited to test fixtures; regular Litty was untouched.

## Moved-Tab UI Check

The uninstrumented fixture /tmp/litty-moved-tab-a0ds3p4m ran as process 91964.
Its first pane SOURCE A emitted `Litty moved-tab #1`; subsequent panes were
labeled KEEP TAB B and TARGET WINDOW C. The instructed flow was Cmd+T to keep
the original window alive, Cmd+N to create C in the same process, drag SOURCE A
to C, select C's other tab, then click the notification. The maintainer
confirmed `ok tested, very good`. Record this moved-tab selection/foreground
flow as user-confirmed passed. No diagnostic library or production code change
was involved. This does not cover a zoomed split or the revised cold-start
policy; those remain separate checks/work.

The fixture counter is 4 (the maintainer created an additional pane beyond the
three labeled setup panes). Native stderr contains AppKit's `Warning: Window
move completed without beginning`. This warning was not diagnosed; the
maintainer reported the requested UI flow succeeded. Do not claim stderr was
empty or attribute the warning to an unrelated cause without evidence.

## Zoomed-Split UI Check

The uninstrumented fixture /tmp/litty-zoomed-split-x8capp_q ran as PID 92575.
ZOOM SOURCE A emitted `Litty zoomed-split #1`; Cmd+D created ZOOM PANE B,
and Cmd+Shift+Enter zoomed B to hide A. The instructed flow then switched to
another app and clicked the original notification, expecting the test window
to come forward, B's zoom to clear and A to be selected without creating a
pane/window. The maintainer confirmed `ok tested, very good`. Record this
zoomed-split reveal/selection/foreground check as user-confirmed passed.
No production source change or diagnostic library was involved.
Native stderr again contains `Warning: Window move completed without beginning`
(twice). This is the same undiagnosed AppKit warning recorded above; no cause
or fix is asserted based on the successful UI outcome.

## Dock/Menu UI Check

The numbered, uninstrumented fixture /tmp/litty-dock-menu-qak5o59v ran as
PID 93139. The requested flow closed the last test window with its red button,
clicked the test app's Dock icon to reopen, closed that window, then used
Cmd+N while the app remained active. The maintainer reported `tested good`.
Record the manual Dock/menu lifecycle flow as user-confirmed passed. The
fixture counter reached 5, showing additional window creations beyond the
three-window initial script; the counter alone does not establish which
individual action created each window or prove one window per action.

Native stderr contains the previously recorded, undiagnosed AppKit warning
`Window move completed without beginning`. No production source was changed
for this manual flow. The revised cold-start implementation/test policy,
old intermittent reports, real-ID authorization and review/runtime limits
remain separate from these successful probe-identity UI checks.
