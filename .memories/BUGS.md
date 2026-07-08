# BUGS
> Known defects and their status. Last updated: 2026-07-08. **No open bugs.**

## `[FIXED 2026-07-08]` compose outputs were hashed by tile index, not content

**Fix:** compose ops now build pixels with throwaway index hashes (a private `blend`), then
**re-stamp** the assembled tiles with a content-true signature via `compose_signature(params,
input_tile_hashes…)` + `global_tile_hash` (new `craws/cmp` hash domain). `overlay` folds
(base+top+x/y/opacity), `collage` folds (all original images + options), `diff.side_by_side` folds
(a+b+gap). Distinct inputs/params ⇒ distinct identities, so no downstream cache collision.
Regression tests: `compose::tests::compose_outputs_are_content_addressed` (identity-level) and
`engine::tests::compose_outputs_dont_poison_the_cache` (end-to-end: two overlays → exposure each →
results must differ). Both fail on the old code, pass now.

<details><summary>Original report (kept for the record)</summary>

**Where:** `crates/craws-engine/src/compose.rs` — `hash_by_index(i) = digest_bytes(i.to_le_bytes())`
is used as the `tile_hash` for `overlay`, `collage`, and `diff`'s `side_by_side` layout.

**The hole:** every compose output of a given grid size gets the SAME per-tile identities regardless of
its pixels. The pixels themselves are safe — the session holds each result's `Arc<Tile>` in its handle
map, so `export`/`image_info` are correct. The danger is downstream: a **cached single-input `OpSpec`
run on two different compose outputs of equal grid dimensions** derives identical cache keys
(`pointwise_tile_hash` / `global_signature` fold the input tile hashes, which collide), so the second
run can be served the first's cached result.

**Repro (not yet turned into a test):**
1. `overlayA = overlay(base1, top, …)`, `overlayB = overlay(base2, top, …)` — same size, different content.
2. `exposure(overlayA, +1)` (computes + caches), then `exposure(overlayB, +1)`.
3. Step 2's second call hits overlayA's cached exposed tiles → overlayB gets the wrong pixels.

**Severity:** latent correctness. Needs (a) ≥2 compose results of equal grid size **and** (b) a cached
op applied on top **and** (c) a shared engine (the session always shares one). Plausible in a docs
workflow (overlay a badge on two shots, then tweak both).

**Safe by contrast:**
- `trim` (2026-07-08) derives tile hashes from `tag(rect) ⊕ input tile hashes` — content-addressed.
- `diff`'s `difference`/`heatmap` build via `from_srgb_rgba8(size, &out, digest_bytes(&out))` — the
  source digest is the actual pixels, so identities are content-true.
- All `OpSpec` ops go through `run_global`/`run_pointwise`, which hash from op params + real input
  identities — never index-only.

**Fix direction (this is what was implemented):** give compose outputs content-true identities —
derive a signature from the input images' tile hashes + op params (as `run_global` does) and use
`global_tile_hash(sig, i)`. Done across overlay/collage/side_by_side via one `stamp` helper.

</details>
