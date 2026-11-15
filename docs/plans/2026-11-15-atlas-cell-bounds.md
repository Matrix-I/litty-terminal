# Atlas Cell Bounds Implementation Plan
> **For agentic workers:** REQUIRED SUB-SKILL: executing-plans-strict.
**Goal:** Prevent Atlas division by zero and scratch-size overflow from extreme cell adjustments.
**Architecture:** Clamp final metrics before geometry or allocation, in shared macOS/Linux code.
**Tech Stack:** Rust, native GPU backend, existing Atlas tests.
**Branch:** fix/atlas-cell-bounds

## Confirmed Requirements
- User chose automatic clamping rather than rejecting config values.
- Fix Atlas only; retain font-size configuration and zoom semantics.
## Global Constraints
- Preserve 12-byte Cell, platform parity, no blocking I/O under grid locks or new hot-path allocations. No dependencies, public signatures or config format changes.
- Commit only when asked. No push/release.
- Run cargo check; cargo check --target x86_64-unknown-linux-gnu; cargo test outside sandbox; cargo clippy --all-targets -- -D warnings; git diff --check.
## Technical Defaults
- Maximum cell side: 2048, half the existing 4096 atlas limit, guaranteeing four slots (blank plus distinct wide-glyph halves).
- Clamp in MetricAdjust::apply, used by logical measurement and GPU construction. Keep normal values unchanged.
- Fixed bounds prove existing u32 scratch products safe: 2048 * 2048 * 8 = 33554432. Add compile-time assertion to preserve that proof.
- Private constructor accepts adjustments for deterministic GPU tests without global mutation; public constructor snapshots CELL_ADJUST and delegates.
## Risks & Rollback
- Extreme configs render with clamped dimensions; document the cap. Four slots can thrash for extreme configs, but stay bounded. Revert this isolated diff; no persisted format or API migration.
- Native GPU boundary tests on macOS; Linux compile check and shared pure tests cover platform-independent logic. Report any missing Linux GPU run.

## Task 1 — Clamp metrics, regression coverage and verification
**Files:** src/render/atlas/mod.rs (MetricAdjust::apply and Atlas::new); src/render/atlas/tests.rs (metric adjustment and GPU tests); README.md; docs/guide/configuration.md; docs/plans/0.13.0.md; docs/plans/2026-11-13-verified-findings-fixes.md.
**Interfaces:** Preserve public signatures. Private new_with_adjustments takes the existing four constructor arguments plus (MetricAdjust, MetricAdjust).
- [x] Add failing pure tests: apply Px(4097), Px(i32::MAX), Percent(f32::MAX), None with u32::MAX => 2048; negative extremes => 1; normal values unchanged.
- [x] Run cargo test metric_adjust_caps; expect assertion failures (values exceed 2048).
- [x] Add MAX_ATLAS_SIDE = 4096, MAX_CELL_SIDE = MAX_ATLAS_SIDE / 2 and a compile-time u32 product bound. Implement (adjusted.round().clamp(1.0, MAX_CELL_SIDE as f32) as u32).max(1), retaining one pixel for direct internal NaN values. Reuse MAX_ATLAS_SIDE in atlas sizing/growth.
- [x] Extract private new_with_adjustments; public new snapshots global tweaks and delegates. Add GPU test with Px(i32::MAX), Percent(f32::MAX): dimensions 2048, slots 4, scratch sizes exact; glyph_slots for a wide glyph returns distinct nonblank slots, repeat lookup matches. Exercise rendering uploads. Add lower-bound and unchanged normal unit coverage.
- [x] Document clamping in README/config guide and close only the Atlas item in v0.13.0/verified findings plan, preserving original evidence and other open tasks. Font zoom consistency inspected but intentionally unchanged.
- [x] Run full verification commands; read all diffs/untracked files. Self-review requirements/edges, concurrency, errors, performance, security, naming, assumptions, tests and design with reasons.
- [x] Apply code-review-strict, triage and rerun gates after fixes. Report runtime gaps and unavailable reviewer requirements.
**Ready to commit:** files above and this plan — `fix(render): clamp atlas cell dimensions` (no commit authorized).

## Review
Single task, no API/persisted format changes. All requested behavior maps to clamp regression and native GPU construction/upload checks. Current-base standalone extracted calculation reproduced divide-by-zero on cc80d87 with adjust-cell-width=4097 (exit 101); full GUI not run before change.

## Verification and self-review (2026-11-15)
- macOS arm64, baseline cc80d87. Original extracted calculation: Px(4097) divides by zero, exit 101. Same calculation after fix: cell_w=2048, slots_per_row=2, n_slots=408; debug and optimized panic=abort builds exit 0.
- Red: cargo test metric_adjust_caps failed (4114 != 2048). Green: both new tests pass on native Metal outside sandbox, including wide-glyph uploads and cache reuse.
- cargo check: PASS; cargo check --target x86_64-unknown-linux-gnu: PASS; cargo clippy --all-targets -- -D warnings: PASS.
- cargo test outside sandbox: PASS, 709 unit tests + 1 DSR test + 14 shell tests, 12 ignored; native activation harness PASS.
- First full run failed the new test's oversized literal and render-to-config test import. Corrected the test to parse a finite exponent via MetricAdjust directly; next full run passed. A focused sandbox run could not create a Metal device; unsandboxed full suite passed.
- git diff --check: PASS. Full tracked diff and untracked plan read; no unrelated edits, debug output or secrets.
- Requirements/edges: pure tests cover saturation, negative extremes and unchanged normal values; native GPU test asserts actual four-slot geometry and all buffer lengths.
- Concurrency: tweaks are copied under the existing mutex, released before font/GPU work; tests inject tweaks without global mutations and hold TEST_GPU.
- Errors/security: finite config parser unchanged; final bound removes divide-by-zero and u32 overflow. No external I/O, new permissions, shell interpolation or persisted formats.
- Performance: scalar clamp at atlas measurement/construction only; fixed scratch bound, no new render/parser allocations or grid-lock work. Extreme configs can thrash the four-slot atlas; normal dimensions preserve geometry.
- Naming/APIs/design: private constructor delegates shared implementation; public signatures unchanged; MAX_ATLAS_SIDE replaces only related size limits; no dependency or API updates.
- Business rules: user explicitly chose clamping; 2048 follows the four-slot capacity requirement. Font-size startup/reload/zoom consistency reviewed and unchanged as out of scope.
- End-to-end evidence: native GPU constructor and glyph uploads exercised, with config value parsing. Interactive app/config reload and Linux GPU runtime not exercised; cannot claim those passed. No user config was modified for testing.

## Independent review disposition
- Opus CLI could not run: sandbox DNS ENOTFOUND; escalated attempt returned Credit balance is too low. The user subsequently authorized Sol in place of Opus for this session; Sol review is the accepted model requirement.
- Round 1: Approved, zero Critical/Important; one confirmed Minor: direct Percent(NaN) changed from one pixel to zero, potentially recreating division-by-zero for internal construction. Config parser already rejects NaN. Fixed with final integer max(1). New test failed 0 != 1 before the fix; asserts NaN => 1 and infinities => bounded endpoints after it.
- Round 2 approved the final code fix with no remaining findings after verification. A final explicit Sol review was requested under the user-authorized model substitution before opening the PR. Linux GPU runtime and interactive config reload remain unverified.

## Concrete implementation and regression tests

The complete code delta for Task 1 (including the review correction):

```diff
diff --git a/src/render/atlas/mod.rs b/src/render/atlas/mod.rs
index 84f2f69..b563461 100644
--- a/src/render/atlas/mod.rs
+++ b/src/render/atlas/mod.rs
@@ -42,6 +42,12 @@ use linux as system;
 #[cfg(all(test, target_os = "macos"))]
 pub(crate) use coretext::macos_coretext_rasterize;

+const MAX_ATLAS_SIDE: u32 = 4096;
+// Reserve at least four slots: blank and both halves of a wide glyph.
+const MAX_CELL_SIDE: u32 = MAX_ATLAS_SIDE / 2;
+// The largest scratch buffer holds two RGBA cells; all size products fit u32.
+const _: () = assert!(MAX_CELL_SIDE as u64 * MAX_CELL_SIDE as u64 * 8 <= u32::MAX as u64);
+
 /// `adjust-cell-width` / `adjust-cell-height`: grow or shrink the cell by
 /// pixels or by a percentage of the font's own metric.
 #[derive(Copy, Clone, Debug, Default, PartialEq)]
@@ -69,7 +75,8 @@ impl MetricAdjust {
             MetricAdjust::Px(px) => v as f32 + px as f32,
             MetricAdjust::Percent(p) => v as f32 * (1.0 + p / 100.0),
         };
-        adjusted.round().max(1.0) as u32
+        // Direct enum construction can supply NaN, whose integer cast is zero.
+        (adjusted.round().clamp(1.0, MAX_CELL_SIDE as f32) as u32).max(1)
     }
 }

@@ -242,12 +249,22 @@ impl Atlas {
         queue: &crate::render::backend::Queue,
         fonts: Arc<FontSet>,
         px: f32,
+    ) -> Self {
+        let adjustments = *CELL_ADJUST.lock();
+        Self::new_with_adjustments(device, queue, fonts, px, adjustments)
+    }
+
+    fn new_with_adjustments(
+        device: &crate::render::backend::Device,
+        queue: &crate::render::backend::Queue,
+        fonts: Arc<FontSet>,
+        px: f32,
+        (adjust_w, adjust_h): (MetricAdjust, MetricAdjust),
     ) -> Self {
         let font = &fonts.primary;
         let lm = line_metrics(font, px);
         let natural_w = font.advance_width('M', px).round().max(1.0) as u32;
         let natural_h = (lm.ascent - lm.descent + lm.line_gap).ceil().max(1.0) as u32;
-        let (adjust_w, adjust_h) = *CELL_ADJUST.lock();
         let cell_w = adjust_w.apply(natural_w);
         let cell_h = adjust_h.apply(natural_h);
         // Extra (or missing) height is split above and below the text.
@@ -255,7 +272,7 @@ impl Atlas {

         // 512×512 default (256KB VRAM for R8, 1MB for RGBA8); grows dynamically for large fonts.
         let mut size = 512u32;
-        while (size / cell_w) * (size / cell_h) < 256 && size < 4096 {
+        while (size / cell_w) * (size / cell_h) < 256 && size < MAX_ATLAS_SIDE {
             size *= 2;
         }
         let slots_per_row = size / cell_w;
@@ -311,10 +328,10 @@ impl Atlas {
         device: &crate::render::backend::Device,
         queue: &crate::render::backend::Queue,
     ) -> bool {
-        if !std::mem::take(&mut self.grow_needed) || self.size >= 4096 {
+        if !std::mem::take(&mut self.grow_needed) || self.size >= MAX_ATLAS_SIDE {
             return false;
         }
-        let new_size = (self.size * 2).min(4096);
+        let new_size = (self.size * 2).min(MAX_ATLAS_SIDE);
         let slots_per_row = new_size / self.cell_w;
         let n_slots = (slots_per_row * (new_size / self.cell_h)).min(u16::MAX as u32);
         if n_slots <= self.n_slots {
diff --git a/src/render/atlas/tests.rs b/src/render/atlas/tests.rs
index 1f94f82..efdb810 100644
--- a/src/render/atlas/tests.rs
+++ b/src/render/atlas/tests.rs
@@ -173,6 +173,49 @@ fn sized_fonts_keep_several_sizes() {
     assert_eq!(font.cached_sizes().len(), 4, "bounded");
 }

+#[test]
+fn metric_adjust_caps_extreme_cell_dimensions() {
+    for adjust in [MetricAdjust::Px(4097), MetricAdjust::Px(i32::MAX), MetricAdjust::Percent(f32::MAX)] {
+        assert_eq!(adjust.apply(17), 2048, "{adjust:?}");
+    }
+    assert_eq!(MetricAdjust::None.apply(u32::MAX), 2048);
+    assert_eq!(MetricAdjust::Px(i32::MIN).apply(17), 1);
+    assert_eq!(MetricAdjust::Percent(-f32::MAX).apply(17), 1);
+    for size in [1, 17, 2047, 2048] {
+        assert_eq!(MetricAdjust::None.apply(size), size);
+    }
+}
+
+#[test]
+fn metric_adjust_keeps_nonfinite_internal_values_nonzero() {
+    assert_eq!(MetricAdjust::Percent(f32::NAN).apply(17), 1);
+    assert_eq!(MetricAdjust::Percent(f32::INFINITY).apply(17), 2048);
+    assert_eq!(MetricAdjust::Percent(f32::NEG_INFINITY).apply(17), 1);
+}
+
+#[test]
+fn atlas_caps_extreme_cell_dimensions_and_uploads_wide_glyphs() {
+    let _gpu = crate::render::backend::TEST_GPU.lock();
+    let (device, queue) = crate::render::backend::shared_device_and_queue();
+    let mut atlas = Atlas::new_with_adjustments(
+        &device, &queue, load_font(&[]), 16.0,
+        (MetricAdjust::parse("2147483647").unwrap(), MetricAdjust::parse("3.4028235e38%").unwrap()),
+    );
+    assert_eq!((atlas.cell_w, atlas.cell_h), (2048, 2048));
+    assert_eq!((atlas.size, atlas.slots_per_row, atlas.n_slots), (4096, 2, 4));
+    assert_eq!(atlas.scratch.len(), 2048 * 2048);
+    assert_eq!(atlas.color_scratch.len(), 2048 * 2048 * 4);
+    assert_eq!(atlas.wide_r8_scratch.len(), 2048 * 2048 * 2);
+    assert_eq!(atlas.wide_rgba_scratch.len(), 2048 * 2048 * 8);
+    atlas.begin_frame();
+    let slots = atlas.glyph_slots(&queue, '中' as u32, 2, GlyphStyle::default());
+    assert_ne!(slots.0, BLANK_SLOT);
+    assert_ne!(slots.1, BLANK_SLOT);
+    assert_ne!(slots.0, slots.1);
+    assert_eq!(atlas.glyph_slots(&queue, '中' as u32, 2, GlyphStyle::default()), slots);
+    assert!(!atlas.grow_if_needed(&device, &queue), "already at the texture limit");
+}
+
 #[test]
 fn metric_adjust_parses_pixels_and_percent() {
     assert_eq!(MetricAdjust::parse("2"), Ok(MetricAdjust::Px(2)));
```
