# BUGS
> Known defects and their status. Last updated: 2026-07-08.

## `[BUG]` compose outputs are hashed by tile index, not content

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

**Fix direction (separate focused task, not done):** give compose outputs content-true identities —
e.g. derive a signature from the input images' tile hashes + op params (as `run_global` does) and use
`global_tile_hash(sig, i)`, or route compose through the same signature machinery. Touches
overlay/collage/side_by_side together. Add the repro above as a regression test when fixing.
