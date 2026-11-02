# Release Process

Litty is released through **GitHub Releases** at
<https://github.com/Matrix-I/litty-terminal/releases>. The built-in updater
(`src/update.rs`) reads those releases directly, so asset names and the
checksum manifest are part of the contract — do not rename them casually.

> Agents: every step that pushes, tags, or publishes requires explicit user
> approval in the current session.

## 1. Versioning

- [Semantic Versioning](https://semver.org/): `MAJOR.MINOR.PATCH`, tags are
  `vX.Y.Z` (e.g. `v0.6.0`).
  - `PATCH` — bug/security fixes only, no config or keybinding changes.
  - `MINOR` — new features, new config keys, roadmap milestones.
  - `MAJOR` — breaking config format or behavior changes (post-1.0).
- Pre-releases use `vX.Y.Z-rc.N` and are published with `--prerelease`
  (only the `tip` update channel picks them up).

## 2. Prepare (on `main`, clean working tree)

1. Make sure every change intended for the release is merged into `main` and
   `git status` is clean.
2. Bump the version in **all** of:
   - `Cargo.toml` → `version`
   - `resources/Info.plist` → `CFBundleShortVersionString` and `CFBundleVersion`
   - `Cargo.lock` (run `cargo check` to refresh it)
   - `README.md` → the **version** badge
3. Update `ROADMAP.md`: mark completed items, move unfinished ones to the next
   milestone.
4. Draft release notes in `dist/vX.Y.Z/RELEASE_NOTES.md` (template below).
5. Commit: `chore(release): prepare vX.Y.Z`.

> The measured README badges (**idle CPU**, **idle RAM**, **binary size**) and
> the table in `docs/development/performance.md` must match the numbers of the release
> being shipped. Refresh them in §3 once `scripts/pre-release-check.sh` has
> produced fresh figures; never ship a release with stale badges.

## 3. Verify (release blockers)

Run on macOS arm64, outside the sandbox:

```bash
cargo check
cargo check --target x86_64-unknown-linux-gnu
cargo clippy --all-targets -- -D warnings
cargo test
scripts/pre-release-check.sh
```

- `scripts/pre-release-check.sh` must exit 0. It writes
  `scripts/release-test-report.md`; copy it to `dist/vX.Y.Z/VERIFICATION.md`.
- Update the README badges **idle CPU**, **idle RAM** and **binary size**, and
  the Litty row of `docs/development/performance.md`, from the report's numbers and the
  size of the release binary. If a figure could not be measured, leave the old
  value only after noting it under **Known limitations**. Fold the change into
  the release commit (amend before tagging, or a `docs(readme)` commit).
- Linux runtime smoke test (when Docker/Podman is available):
  `scripts/linux/test.sh` — see `scripts/linux/README.md`.
- Record anything that could not be run (e.g. Linux runtime, idle CPU) under
  **Known limitations** in the release notes. Never claim an unrun check passed.

## 4. Build artifacts

All outputs go into `dist/vX.Y.Z/` (`dist/` is gitignored).

```bash
VERSION=vX.Y.Z
OUT=dist/$VERSION
mkdir -p "$OUT"
```

### macOS (Apple Silicon)

```bash
./bundle_macos.sh --no-install
cp dist/Litty.dmg dist/Litty-macos-aarch64.zip dist/litty-macos-aarch64 "$OUT/"
```

### Linux (x86_64, glibc 2.34 baseline)

Cross-compiled with `cargo-zigbuild` (`brew install zig cargo-zigbuild`):

```bash
LITTY_MINISIGN_PUBKEY="$(tail -n 1 resources/minisign.pub)" \
  cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.34
cp target/x86_64-unknown-linux-gnu/release/litty "$OUT/litty-linux-x86_64"
tar -C "$OUT" -czf "$OUT/litty-linux-x86_64.tar.gz" litty-linux-x86_64
```

### Required asset names

The updater (`asset_candidates` in `src/update.rs`) looks for exactly these:

| Asset | Purpose |
|---|---|
| `Litty-macos-aarch64.zip` | macOS `.app` bundle (updater, app install) |
| `litty-macos-aarch64` | macOS standalone binary (updater, CLI install) |
| `Litty.dmg` | macOS drag-and-drop installer (manual download) |
| `litty-linux-x86_64` | Linux binary (updater) |
| `litty-linux-x86_64.tar.gz` | Linux archive (manual download) |
| `SHA256SUMS.txt` | Checksums of every asset above |
| `SHA256SUMS.txt.minisig` / `.asc` | Signature over the checksums (see §5) |
| `VERIFICATION.md` | Pre-release suite report |

## 5. Checksums and signing

```bash
(cd "$OUT" && shasum -a 256 Litty.dmg Litty-macos-aarch64.zip litty-macos-aarch64 \
  litty-linux-x86_64 litty-linux-x86_64.tar.gz > SHA256SUMS.txt)
(cd "$OUT" && shasum -a 256 -c SHA256SUMS.txt)
```

The updater **fails closed** on releases without a signature it can verify
against a key pinned at build time (`LITTY_MINISIGN_PUBKEY` or
`LITTY_GPG_FINGERPRINT`). `bundle_macos.sh` pins the key from
`resources/minisign.pub` unless `LITTY_MINISIGN_PUBKEY` is set, and the Linux
command in §4 passes the same key.

`bundle_macos.sh` also signs a macOS-only `dist/SHA256SUMS.txt`; the release
needs the combined file above signed instead:

```bash
minisign -Sm "$OUT/SHA256SUMS.txt"     # produces SHA256SUMS.txt.minisig
minisign -Vm "$OUT/SHA256SUMS.txt" -p resources/minisign.pub
```

Signing keys never enter the repository. If a release is shipped unsigned,
say so under **Known limitations** (users must update manually).

## 6. Tag and publish

1. Push the release commit: `git push origin main`.
2. Create an annotated tag and push it:

   ```bash
   git tag -a "$VERSION" -m "Litty $VERSION"
   git push origin "$VERSION"
   ```

3. Publish the release:

   ```bash
   gh release create "$VERSION" \
     --title "Litty $VERSION" \
     --notes-file "$OUT/RELEASE_NOTES.md" \
     "$OUT"/Litty.dmg "$OUT"/Litty-macos-aarch64.zip "$OUT"/litty-macos-aarch64 \
     "$OUT"/litty-linux-x86_64 "$OUT"/litty-linux-x86_64.tar.gz \
     "$OUT"/SHA256SUMS.txt "$OUT"/VERIFICATION.md
   ```

   Add `"$OUT"/SHA256SUMS.txt.minisig` when signed; add `--prerelease` for
   `-rc.N` tags. Use `--draft` first if you want to review the page before it
   goes live.

## 7. Post-release checks

- `gh release view "$VERSION"` lists every asset from §4.
- Download `SHA256SUMS.txt` plus one asset from the release page and run
  `shasum -a 256 -c` against it.
- On an older installed build, run the update check and confirm it sees the
  new version (and, if signed, installs it).
- Open the next milestone in `ROADMAP.md`.

## Hotfix releases

Branch from the release tag (`git switch -c fix/vX.Y.Z+1 vX.Y.Z`), apply only
the fix with a regression test, then follow §2–§7 with a `PATCH` bump. Merge
the branch back into `main` afterwards.

## Release notes template

````markdown
# Litty vX.Y.Z

<one-paragraph summary of the release theme>

## ⚠️ macOS Gatekeeper Note (Apple Silicon)

Litty is ad-hoc signed and is not yet notarized by Apple, so macOS Gatekeeper may block it on first launch.

After copying `Litty.app` into `/Applications`, open Terminal and run the following command for this downloaded app:

```bash
xattr -dr com.apple.quarantine /Applications/Litty.app
```

Then launch Litty normally from Spotlight or the Applications folder.

---

## Changes

- <user-visible change, grouped by area>

## Validation

Checked on <platform>; release commit <short sha>:

- cargo check / Linux target check / clippy: <result>
- cargo test: <N passed, M ignored>
- Pre-release suite: <result + key numbers (idle RAM, PTY P99, throughput)>
- <what was NOT run and why>

## Downloads

- **macOS Apple Silicon:** `Litty.dmg` or `Litty-macos-aarch64.zip`.
- **macOS standalone executable:** `litty-macos-aarch64`.
- **Linux x86_64:** `litty-linux-x86_64` or its `.tar.gz`; requires glibc 2.34+.
- Verify integrity with `SHA256SUMS.txt`.

## Known limitations

- <unsigned release, unmet targets, untested platforms, …>
````
