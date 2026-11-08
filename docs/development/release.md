# Release Process

Litty is released through **GitHub Releases** at
<https://github.com/Matrix-I/litty-terminal/releases>. The built-in updater
(`src/update.rs`) reads those releases directly, so asset names and the
checksum manifest are part of the contract — do not rename them casually.

> Agents: every step that pushes, tags, or publishes requires explicit user
> approval in the current session.

## Automated release (CI)

Pushing a `vX.Y.Z` or `vX.Y.Z-rc.N` tag runs `.github/workflows/release.yml`,
which replaces §4–§5 and step 3 of §6:

1. Checks the tag against `Cargo.toml` and `resources/Info.plist`.
2. Builds the macOS assets on `macos-15` (after `cargo test --release`) and the
   Linux binary with `cargo-zigbuild` (glibc 2.34), both with the key from
   `resources/minisign.pub` pinned.
3. Builds and tests the `.deb` (Debian 13) and `.rpm` (Fedora 43), including
   an install/remove round trip.
4. Checks the key and version are in both binaries, writes `SHA256SUMS.txt`,
   signs it, verifies the signature against `resources/minisign.pub`, and
   creates a **draft** release with every asset.

§2 and §3 still run locally before tagging: the pre-release suite and the idle
RAM/CPU figures need a real Mac GUI session. Afterwards, replace the draft's
generated notes with `RELEASE_NOTES.md`, review it, and publish it (§7 checks
still apply).

One-time setup (repository **Settings**):

- **Environments → New environment `release`**: add yourself as a required
  reviewer and restrict deployment to tags matching `v*`. The signing job waits
  for that approval.
- In that environment, add the secrets `MINISIGN_SECRET_KEY` (the full content
  of the secret key file) and `MINISIGN_PASSWORD` (empty if the key has none).
- **Rules → Rulesets**: protect `v*` tags and `.github/workflows/` changes so
  only reviewed changes can reach the signing job.

If the key may have leaked, every installed build needs a manual update to a
build that pins the new key, so rotate it only with a release planned for it.

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
5. Commit: `chore(release): prepare vX.Y.Z`. Stage files explicitly; the
   release commit must not contain unrelated working-tree changes.

The release commit lands on `main` (an exception to the branch rule for
non-trivial work, since it only bumps versions, badges and docs). Once the
release is published, set the **Status** line of `docs/plans/X.Y.Z.md` to
"released" with the date and the `ROADMAP.md` row to a historical record.

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
  `scripts/release-test-report.md`; archive it in
  `docs/validation/release-X.Y.Z.md` and link that committed file in the release
  notes. Keep packaging reports in `docs/validation/` too; do not upload reports
  as release assets.
- Update the README badges **idle CPU**, **idle RAM** and **binary size**, and
  the Litty row of `docs/development/performance.md`. The suite checks budgets
  only and does not report idle CPU or idle RAM, so measure those yourself
  (macOS `footprint` with the app idle and all windows closed). Binary size is
  the byte size of the release binary in decimal MB (`ls -l`, divided by
  1,000,000, one decimal). If a figure could not be measured, leave the old
  value only after noting it under **Known limitations**. Fold the change into
  the release commit (amend before tagging, or a `docs(readme)` commit).
- Fill the release commit SHA in `RELEASE_NOTES.md` and the archived validation report
  only after the last amend, since amending changes the SHA. If code changed
  after the checks ran, rerun them on the final commit.
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
```

### Debian package (amd64)

On Debian (or in an isolated Debian container with `dpkg`, `binutils`, and
`desktop-file-utils`), package the same executable without rebuilding it:

```bash
bash scripts/linux/package-deb.sh "$OUT/litty-linux-x86_64" "${VERSION#v}" "$OUT"
bash scripts/linux/test-package-deb.sh "$OUT/litty-linux-x86_64"
```

Install the resulting `.deb` in a clean amd64 Debian test environment with
`apt install ./litty_<version>_amd64.deb`. Launch the installed desktop entry and
verify a terminal window opens; remove it with `apt remove litty` and verify the
launcher/icon are removed. Record any missing GUI validation explicitly.

### RPM package (x86_64)

On Fedora with `rpm-build`, `binutils`, `cpio`, and `desktop-file-utils`:

```bash
bash scripts/linux/package-rpm.sh "$OUT/litty-linux-x86_64" "${VERSION#v}" "$OUT"
bash scripts/linux/test-package-rpm.sh "$OUT/litty-linux-x86_64"
```

Test installation/removal through DNF in a clean Fedora amd64 environment and
exercise the installed desktop launcher. Preserve and report any GUI validation
gap. This RPM wraps the existing executable; do not disable automatic dependency
detection or modify/strip the binary during packaging. The RPM is not RPM-signed;
its integrity is covered by the signed release checksum manifest.

### Verify the artifacts

Do not run the binary with `--version`: it starts the GUI. Inspect instead:

```bash
VER=${VERSION#v}
unzip -p "$OUT/Litty-macos-aarch64.zip" Litty.app/Contents/Info.plist \
  | plutil -extract CFBundleShortVersionString raw -o - -   # expect $VER
KEY="$(tail -n 1 resources/minisign.pub)"
for f in litty-macos-aarch64 litty-linux-x86_64; do
  strings -a "$OUT/$f" | grep -Fq "$KEY" && echo "$f: key pinned" || echo "$f: KEY MISSING"
  strings -a "$OUT/$f" | grep -Fq "$VER" && echo "$f: version ok" || echo "$f: VERSION MISSING"
done
```

A binary without the pinned key refuses every auto-update, so a `KEY MISSING`
result blocks the release.

### Required asset names

The release uses these asset names. The updater selects the standalone Linux
executable or macOS app/standalone build; `.deb` and `.rpm` are manual
package-manager installs:

| Asset | Purpose |
|---|---|
| `Litty-macos-aarch64.zip` | macOS `.app` bundle (updater, app install) |
| `litty-macos-aarch64` | macOS standalone binary (updater, CLI install) |
| `Litty.dmg` | macOS drag-and-drop installer (manual download) |
| `litty-linux-x86_64` | Linux binary (updater) |
| `litty_<version>_amd64.deb` | Debian/Ubuntu package with desktop launcher and icon |
| `litty-<version>-1.x86_64.rpm` | Fedora RPM with desktop launcher and icon |
| `SHA256SUMS.txt` | Checksums of every asset above |
| `SHA256SUMS.txt.minisig` / `.asc` | Signature over the checksums (see §5) |

Keep this list to eight uploaded assets: three installers (`.dmg`, `.deb`,
`.rpm`), three updater builds (macOS app ZIP, macOS standalone, Linux standalone),
and the checksum manifest plus signature. The app ZIP and standalone builds are
required by existing updater clients, even though installers are the primary
manual downloads. Do not also publish a Linux archive containing the same binary.
GitHub adds two source archives automatically; those are not uploaded assets.

## 5. Checksums and signing

```bash
(cd "$OUT" && shasum -a 256 Litty.dmg Litty-macos-aarch64.zip litty-macos-aarch64 \
  litty-linux-x86_64 "litty_${VERSION#v}_amd64.deb" "litty-${VERSION#v}-1.x86_64.rpm" > SHA256SUMS.txt)
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
     --title "Litty $VERSION ⚡" \
     --notes-file "$OUT/RELEASE_NOTES.md" \
     "$OUT"/Litty.dmg "$OUT"/Litty-macos-aarch64.zip "$OUT"/litty-macos-aarch64 \
     "$OUT"/litty-linux-x86_64 "$OUT"/litty_${VERSION#v}_amd64.deb "$OUT"/litty-${VERSION#v}-1.x86_64.rpm \
     "$OUT"/SHA256SUMS.txt
   ```

   Add `"$OUT"/SHA256SUMS.txt.minisig` when signed; add `--prerelease` for
   `-rc.N` tags. Use `--draft` first if you want to review the page before it
   goes live.

## 7. Post-release checks

- `gh release view "$VERSION"` lists every asset from §4.
- Download `SHA256SUMS.txt` plus one asset from the release page and run
  `shasum -a 256 -c` against it, and download `SHA256SUMS.txt.minisig` and run
  `minisign -Vm SHA256SUMS.txt -p resources/minisign.pub`.
- On an older installed build, run the update check and confirm it sees the
  new version (and, if signed, installs it).
- Open the next milestone in `ROADMAP.md`.

## Failed or bad release

If publishing fails or a published asset is wrong, mark the release as a
draft or delete it (`gh release delete "$VERSION"`). Delete the tag
(`git push --delete origin "$VERSION"` and `git tag -d "$VERSION"`) only while
no user can have downloaded it. Once a release has been public, never reuse
its tag: ship a new `PATCH` version instead. Agents need explicit approval for
each of these steps.

## Hotfix releases

Branch from the release tag (`git switch -c fix/vX.Y.Z+1 vX.Y.Z`), apply only
the fix with a regression test, then follow §2–§7 with a `PATCH` bump. Merge
the branch back into `main` afterwards.

## Release notes template

Put ⚡ after the version in the GitHub release title and before `Litty` in
the first heading of the release notes.

````markdown
# ⚡ Litty vX.Y.Z

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
- **Linux x86_64:** `litty-linux-x86_64`; requires glibc 2.34+.
- Verify integrity with `SHA256SUMS.txt`.

## Known limitations

- <unsigned release, unmet targets, untested platforms, …>
````
