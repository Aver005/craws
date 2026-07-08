# STATE
> Live project snapshot. Update on every meaningful change.
> Last updated: 2026-07-08 — **toolset broadened 14 → 34 MCP tools** (26 OpSpec variants) across four
> batches: geometry (rotate/flip/pad/trim), color/tone (hue_rotate/invert/brightness_contrast/saturation/
> levels/curves/white_balance/gradient_map), filter (blur/sharpen/vignette/redact/spotlight/beautify),
> compare (diff+metric), meta (run_pipeline). New engine module `filter.rs`. Plus a **fix + BLAZING pass**:
> compose index-hash bug **FIXED** (content-addressed via `stamp`+`compose_signature`; 2 regression tests);
> 16-bit encode LUT (export convert 24MP **106 → 36 ms, ~3×**, no `powf`); tile-direct `blend` + redact
> region reads (no per-pixel `pixel()`); blur tile-row banded/streaming (no full-image flat); beautify
> shadow 1-channel. Tonal ops run in perceptual sRGB (via `ops::map_srgb`, like invert); white_balance/
> vignette in linear. **117 tests pass; `cargo clippy --all-targets -- -D warnings` = 0 across
> domain+engine+mcp+cli** (build with `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUSTFLAGS="-C debuginfo=0"`
> for commit headroom on the page-file-less box). No open bugs (`BUGS.md`). ⚠️ visual eyeball of the new
> ops still owed.

## SNAPSHOT

- **M0 core is live.** `craws run pipeline.json --in a.png --out b.webp` end-to-end: 24MP / 4-step
  pipeline now **~336 ms** (was 556; resize step 307→49 ms). Warm slider tweak on a 24MP chain =
  **11.8 ms**; fully-cached op = **85 µs**.
- **M1 MCP server is live** (`crates/craws-mcp`, rmcp 2.1, stdio, protocol 2024-11-05). **34 tools**:
  - I/O: `open_image` `image_info` `export`
  - transform (OpSpec): `resize` `crop` `rotate` `flip` `pad` `exposure` `grayscale`
  - color/tone (OpSpec, pointwise): `hue_rotate` `invert` `brightness_contrast` `saturation` `levels`
    `curves` `white_balance` `gradient_map`
  - filter (OpSpec): `blur` `sharpen` `vignette` `redact` `spotlight` `beautify`
  - annotation (OpSpec): `draw_rect` `draw_ellipse` `draw_line` `draw_arrow` `draw_text`
  - session-direct (multi-input / content-dependent / meta): `overlay` `collage` `trim` `diff` `run_pipeline`

  Image-handle session: each op returns a NEW immutable `image_id` (mirrors engine tiles), shared
  engine cache across the session. Wired **into pooprusteek** (`%APPDATA%\pooprusteek\mcp.json`,
  entry `craws`; original backed up `.bak-craws`) — new tools appear automatically (schema is discovered
  via `tools/list`).
- **M1 killer feature shipped** (owner's doc-automation use case): annotate screenshots (arrows /
  circles / boxes / text) + **redact** secrets (pixelate/blur/black-bar) + **spotlight** a region +
  one-shot **beautify** (rounded corners + soft shadow + padded background) + **diff** two shots with a
  change metric (visual-regression) + geometry (rotate/flip/pad/**trim** whitespace) + overlay/collage +
  **run_pipeline** (a whole JSON chain in one call). Annotation/compose/geometry verified end-to-end
  over stdio (incl. a pad→trim round-trip and a hue+invert+blur→redact→spotlight→beautify→diff chain);
  text + the new color/filter ops verified by unit tests — a fresh visual eyeball is still owed.
- **Skill**: `skills/craws-mcp/` in the repo is the SOURCE (committed, updated for draw_text). The
  installed copy at `~/.claude/skills/craws-mcp/` is a deployment artifact — **never edit it**.
- **M2 spike is live** (`crates/craws-app` Tauri 2 + `app/` React/WebGPU): real window, 24MP sample,
  exposure slider, pan/zoom, live stats overlay, auto-bench. **The stack decision is now proven by
  measurement, not argument** (see M2 SPIKE VERDICT below).
- Crates: `craws-domain` (types+validation+color; 18 `OpSpec` variants + `FlipAxis`/`RedactMode`),
  `craws-engine` (tiles/hash/cache/ops/resample/**draw** (SDF + spotlight)/**filter** (blur/redact/
  beautify)/**compose** (overlay/collage/diff)), `craws-codecs` (png/jpeg/webp), `craws-cli`
  (bin `craws`), `craws-mcp` (rmcp stdio server + testable `Session`), `craws-app` (Tauri 2).
- Front: `app/` — Vite + React 19 + TS + Tailwind 4 + WebGPU. No animation lib (framer-motion
  rejected). tauri-specta not wired yet (M3).
- UI stack `[DECIDED]`: Tauri 2 + React; animations CSS/WAAPI-only.
- Deferred on purpose: `craws-ops`/`craws-gpu`/`craws-ai`/`craws-project` split out only when the
  boundaries start to hurt — ops live inside the engine for now (see ARCHITECTURE).

## M2 SPIKE VERDICT (2026-07-06, owner's machine, release, 24MP source → 1920px preview = 9.4 MB/frame)

| Path | fps | per-frame breakdown | verdict |
|---|---|---|---|
| pan/zoom (pure GPU transform, no bridge) | **165** | draw only | display-refresh-bound (165 Hz monitor) |
| exposure slider — **authoritative** (engine round-trip every tick) | **7.8** | engine 9 ms · convert 12 ms · **bridge 104 ms** | ❌ fails ≥30 fps; bridge-bound |
| exposure slider — **optimistic** (pointwise op in fragment shader) | **165** | uniform write only; **0 bridge calls** | ✅ the production design |

**Conclusion**: the native-engine/web-shell architecture holds. The bridge (shipping a full 9.4 MB
frame through WebView2 IPC) costs ~104 ms and CANNOT drive an authoritative-per-tick slider — exactly
the risk the plan anticipated. The fix is the planned **optimistic preview**: engine renders a neutral
base once, pointwise ops (exposure/grayscale) run in-shader in linear light during the drag (zero
bridge traffic), engine reconciles once on release. Measured 165 fps = **21× the naive path**, and
it's display-bound, not compute-bound. Exit criteria (PLANS M2): pan/zoom ≥60 ✅, slider ≥30 ✅ (via
optimistic). Design de-risked.

- **Bridge ceiling is a known number now**: ~104 ms / 9.4 MB ≈ 90 MB/s through Tauri IPC in WebView2.
  Fine because pixels rarely need the bridge in production; matters only for ops that can't be
  shader-approximated (blur, resize-on-zoom) — mitigations: changed-tiles-only streaming, mip base.

## MEASURED (2026-07-06, owner's machine, release, 24MP = 6000×4000)

| What | Time |
|---|---|
| CLI end-to-end: PNG decode → 4-step pipeline → WebP write | **336 ms** (was 556) |
| ingest sRGB8 → linear f32 tiles | 45–64 ms |
| export tiles → sRGB8 (24MP) | **~36 ms** (was ~106; 16-bit encode LUT, no `powf`, ~3×) |
| exposure over 24MP, cold / **cached** | 49 ms / **85 µs** |
| resize 6000→1920 Lanczos3, cold | **54 ms** (was 323; own resampler, 6×) |
| blur 24MP σ=8, cold | **~495 ms** (tile-row banded/streaming — no full-image flat copies) |
| redact pixelate 3MP region on 24MP, cold | **~33 ms** (tile-direct region read, no `pixel()`) |
| overlay 2MP top on 24MP | **~18 ms** (tile-direct blend, no per-pixel `pixel()`) |
| chain (resize+exposure+crop+gray), cold / **slider-tweak warm** | ~85 ms / **11.8 ms** |
| decode jpeg / png (24MP) | 128 ms / 157 ms |
| encode jpeg q90 | **375 ms** (was 910; jpeg-encoder SIMD, 2.4×) |
| encode png | 182 ms |

BLAZING-debt pass #1 (2026-07-06): resize + jpeg both attacked and measured. Remaining candidates:
jpeg encode is still single-threaded SIMD — batch parallelism belongs at the port level (encode many
files concurrently) rather than inside one image; png encode 182 ms; export convert (`powf` per
channel) ~106 ms could use an encode LUT.

## DECISIONS LOG

| Date | Decision | Status |
|------|----------|--------|
| 2026-07-06 | Cargo workspace, clean domain core, UI/CLI/MCP as equal ports over `craws-engine` | `[DONE]` |
| 2026-07-06 | Tiles (256², exact-size edges) + Merkle content-hash cache; linear f32 premultiplied | `[DONE]` |
| 2026-07-06 | UI stack: Tauri 2 + React (over GPUI) | `[DECIDED]` |
| 2026-07-06 | BLAZING contract: perf squeezed at every layer, proven by benchmarks | `[DECIDED]` |
| 2026-07-06 | Real-time brush painting OUT of v1 scope | `[DECIDED]` |
| 2026-07-06 | Animations: CSS/WAAPI compositor-only; framer-motion rejected | `[DECIDED]` |
| 2026-07-06 | M0: ops start inside `craws-engine`; global ops (resize) hash at node level | `[DONE]` |
| 2026-07-06 | Text: `DrawText` carries an explicit font PATH (engine stays deterministic); font-NAME→path resolution (fontdb) lives in the port | `[DONE]` |
| 2026-07-06 | Annotation rasterizer is our own SDF in linear light (NOT tiny-skia — would clamp HDR / break invariant) | `[DONE]` |

## BUILD STATUS

| Check | Status |
|-------|--------|
| `cargo build --workspace` | Passes (per-crate; `craws-app` not rebuilt this session — untouched, no exhaustive `match OpSpec`) |
| `cargo test` (per-crate) | **117 passing** (17 domain + 73 engine + 5 codecs + 1+4 cli + 17 mcp-lib/session + 5 mcp-stdio) |
| `cargo clippy --all-targets -- -D warnings` | **0 across domain + engine + mcp + cli** ✅ (gap CLOSED). On this page-file-less box use `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUSTFLAGS="-C debuginfo=0"` for commit headroom |
| Benches | `cargo bench -p craws-engine --bench engine` / `-p craws-codecs --bench codecs` |
| CI | `[DONE]` — **single** `.github/workflows/ci.yml` (staged, no 2nd workflow / no duplicated Build) + `.gitlab-ci.yml` (mirror, already one staged pipeline). Flow: PR / main → `gate` (clippy `-D warnings`·test·bench-compile, **Linux-only**) + `app` (Tauri shell); develop → gate+app → `release-build` (3 OS) → `publish`. **fmt NOT gated.** Rolling `v<ver>-dev` release ships CLI (`craws`+`craws-mcp`) **and** installers (nsis/dmg/deb+appimage). Shared notes: `scripts/dev-release.template.md` + `render-release-notes.sh` |
| Pre-push gate | `[DONE]` — `.githooks/pre-push` (linter·tests·checker, incl. shell build); `git config core.hooksPath .githooks` set; `scripts/install-hooks.sh` re-arms on clone. Bypass: `git push --no-verify` |
| Tauri icons | `[DONE]` — full set in `crates/craws-app/icons/` (placeholder = amber claw marks, `scratchpad/gen_icons.py`); `bundle.icon` wired. Real crab mascot TODO |

## CURRENT FOCUS

M0, M1 (broadened to 26 tools), M2-spike all done. Next candidates (owner's call):
1. **M3 — the editor**: build the real UI on the proven spike (properties panel, linear-chains UI,
   design tokens). Promote spike ad-hoc pieces to infra (tauri-specta, tile streaming, mip base).
2. **Finish the tool menu** — owner is picking from a lettered ~24-option list. DONE (20): A rotate ·
   B flip · C pad · D trim · E brightness_contrast · F saturation · G hue_rotate · H levels · I curves ·
   J white_balance · K invert · L gradient_map · M blur · N sharpen · P vignette · Q redact · R spotlight ·
   S beautify · T diff · U run_pipeline. REMAINING (4): **O** pixelate-standalone (overlaps `redact
   mode:pixelate` — low value) · **V** background_removal (AI, needs `craws-ai`+ort — the big one) ·
   **W** watermark (overlay+text combo) · **X** device_frame (browser/phone chrome mockup).
   Text follow-ups still open: word-wrap (`max_width`), text background/outline, richer shaping.
3. **BLAZING debt** — pass #1 resize 6× + jpeg 2.4×; pass #2 blur/beautify de-alloc; **pass #3
   (2026-07-08): encode LUT (convert ~3×), tile-direct `blend` + redact region reads** (`content_bounds`/
   `trim` were already tile-direct). Remaining: rotate's arbitrary-angle `sample_bilinear` still uses
   `pixel()` (random access — a flat source or tile-cache sampler); `beautify` flattens source+canvas
   (fine for screenshot-sized inputs); port-level batch encode; png encode; a fully-streaming resampler.
   Benched: blur_r8, redact_pixelate, overlay, tiles_to_srgb8; criterion for hue/invert/spotlight owed.

IMMEDIATE: run a fresh visual demo of the new ops (redact/spotlight/beautify/diff on a real screenshot).
