# STATE
> Live project snapshot. Update on every meaningful change.
> Last updated: 2026-07-06 — **M0 + M1 + M2 shipped; first BLAZING-debt pass done**: own separable
> resampler (resize 323→54 ms, 6×, drops `image` dep from engine) + SIMD jpeg encoder (910→375 ms,
> 2.4×). 54 tests, clippy 0.

## SNAPSHOT

- **M0 core is live.** `craws run pipeline.json --in a.png --out b.webp` end-to-end: 24MP / 4-step
  pipeline now **~336 ms** (was 556; resize step 307→49 ms). Warm slider tweak on a 24MP chain =
  **11.8 ms**; fully-cached op = **85 µs**.
- **M1 MCP server is live** (`crates/craws-mcp`, rmcp 2.1, stdio, protocol 2024-11-05). 13 tools:
  `open_image` `resize` `crop` `exposure` `grayscale` `image_info` `export` + **annotation**
  `draw_rect` `draw_ellipse` `draw_line` `draw_arrow` + **composition** `overlay` `collage`. Image-handle
  session: each op returns a NEW immutable `image_id` (mirrors engine tiles), shared engine cache
  across the session. Wired **into pooprusteek** (`%APPDATA%\pooprusteek\mcp.json`, entry `craws`;
  original backed up `.bak-craws`).
- **M1 killer feature shipped** (owner's doc-automation use case): annotate screenshots with
  arrows / circles / boxes (color, stroke width, corner radius, fill, opacity) + overlay screenshots
  + smart auto-layout collages. Verified visually end-to-end over stdio (annotated login mockup +
  3-shot justified collage). Text-on-image is the agreed next iteration.
- **M2 spike is live** (`crates/craws-app` Tauri 2 + `app/` React/WebGPU): real window, 24MP sample,
  exposure slider, pan/zoom, live stats overlay, auto-bench. **The stack decision is now proven by
  measurement, not argument** (see M2 SPIKE VERDICT below).
- Crates: `craws-domain` (types+validation+color), `craws-engine` (tiles/hash/cache/ops/resample/
  **draw** (SDF rasterizer)/**compose** (overlay+collage)), `craws-codecs` (png/jpeg/webp),
  `craws-cli` (bin `craws`), `craws-mcp` (rmcp stdio server + testable `Session`), `craws-app` (Tauri 2).
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
| export tiles → sRGB8 | ~106 ms |
| exposure over 24MP, cold / **cached** | 49 ms / **85 µs** |
| resize 6000→1920 Lanczos3, cold | **54 ms** (was 323; own resampler, 6×) |
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

## BUILD STATUS

| Check | Status |
|-------|--------|
| `cargo build --workspace` | Passes |
| `cargo test --workspace` | **71 passing** (9 domain + 40 engine + 5 codecs + 1+4 cli + 9 mcp-lib/session + 3 mcp-stdio) |
| `cargo clippy --workspace --all-targets` | **0 warnings** |
| Benches | `cargo bench -p craws-engine --bench engine` / `-p craws-codecs --bench codecs` |
| CI | `[DONE]` — **single** `.github/workflows/ci.yml` (staged, no 2nd workflow / no duplicated Build) + `.gitlab-ci.yml` (mirror, already one staged pipeline). Flow: PR / main → `gate` (clippy `-D warnings`·test·bench-compile, **Linux-only**) + `app` (Tauri shell); develop → gate+app → `release-build` (3 OS) → `publish`. **fmt NOT gated.** Rolling `v<ver>-dev` release ships CLI (`craws`+`craws-mcp`) **and** installers (nsis/dmg/deb+appimage). Shared notes: `scripts/dev-release.template.md` + `render-release-notes.sh` |
| Pre-push gate | `[DONE]` — `.githooks/pre-push` (linter·tests·checker, incl. shell build); `git config core.hooksPath .githooks` set; `scripts/install-hooks.sh` re-arms on clone. Bypass: `git push --no-verify` |
| Tauri icons | `[DONE]` — full set in `crates/craws-app/icons/` (placeholder = amber claw marks, `scratchpad/gen_icons.py`); `bundle.icon` wired. Real crab mascot TODO |

## CURRENT FOCUS

M0, M1, M2-spike all done. Next candidates (owner's call):
1. **M3 — the editor**: build the real UI on the proven spike (properties panel, linear-chains UI,
   design tokens). Promote spike ad-hoc pieces to infra (tauri-specta, tile streaming, mip base).
2. **Broaden M1** (annotation + composition DONE 2026-07-06 — draw_rect/ellipse/line/arrow,
   overlay, collage): remaining — **text-on-image** (next iteration, agreed; glyph masks reuse the
   draw.rs coverage→composite path), rotate/flip/blur/brightness-contrast, `run_pipeline` (whole JSON
   in one call), batch-over-folder helper. Watermarking now expressible via overlay + (future) text.
3. **BLAZING debt** (pass #1 done — resize 6× + jpeg 2.4×): remaining — port-level batch parallelism
   (encode many files at once), png encode, export `powf` LUT, and a fully-streaming (no intermediate
   flat) resampler for images that dwarf RAM.
