# PLANS
> Roadmap & active priorities. Last updated: 2026-07-08 (M1+ broadened to 26 tools; menu-driven).

## MILESTONES

### M0 — Core without UI `[DONE 2026-07-06]`
Shipped as planned: workspace of 5 crates, tiled engine + Merkle content-hash cache, ops
(resize/crop/exposure/grayscale), png/jpeg/webp codecs, `craws run` CLI with timing report,
criterion benches. 43 tests, clippy 0. Numbers in `STATE.md` + `JOURNAL/2026-07-06.md`
(headline: warm slider tweak on 24MP chain = 11.8 ms; fully-cached op = 85 µs).
Deviations from plan: `Document` type deferred to M1+ (pipelines don't need it yet);
brightness shipped as photographic `exposure`; curves deferred.

### M1 — MCP server + pooprusteek `[DONE 2026-07-06]` ✅ "it's alive"
`crates/craws-mcp` on rmcp 2.1: stdio, protocol 2024-11-05, tools `open_image` / `resize` / `crop`
/ `exposure` / `grayscale` / `image_info` / `export`. Image-handle session (immutable, new id per op;
shared engine cache). Verified with a real JSON-RPC batch over the spawned binary (2 stdio integration
tests + 5 session unit tests) and wired into pooprusteek's `mcp.json` (entry `craws`, original backed
up). Ran the full open→resize→exposure→grayscale→export batch live.
Deviations from the original plan: tools are one-per-op (chainable via returned image_id) rather than a
single `apply_filter`; `describe_image` deferred (needs vision); the watermark demo needs a compositing
op that doesn't exist yet, so the shipped demo is resize/adjust/convert batch (the real core value).

### M1+ — Broaden: annotation, composition, text `[DONE 2026-07-06]`
Doc-automation toolset over the engine: **annotation** (draw_rect/ellipse/line/arrow — own SDF
rasterizer in linear light) + **composition** (overlay, smart justified-rows collage) + **text**
(draw_text — ab_glyph glyph masks through the same coverage→composite path; embedded Cascadia default;
font-by-name resolved in the port via fontdb so the engine stays deterministic).
Text follow-ups still open: word-wrap, text background/outline, richer shaping.

### M1++ — Broaden further: geometry, color, filter, compare, meta `[DONE 2026-07-08]`
Menu-driven (owner picks lettered options). Landed **12 more tools → 26 total**:
- **geometry** (OpSpec): rotate (90° exact / bilinear+expand), flip, pad; **trim** (auto-crop uniform
  border — session-direct because output size depends on pixels).
- **color** (OpSpec, pointwise): hue_rotate (SVG luma-preserving matrix in linear), invert (perceptual sRGB).
- **filter** (OpSpec, new `filter.rs`): blur (separable Gaussian), redact (pixelate/blur/fill a region),
  spotlight (dim outside a window), beautify (rounded corners + soft shadow + padded background).
- **compare/meta** (session-direct): diff (difference/heatmap/side_by_side + change metric), run_pipeline
  (a whole JSON chain in one call).

Home rule proven twice: single-input & size-computable → `OpSpec` (cached, CLI-usable); multi-input /
content-dependent-size / meta → session-direct function. **109 tests; clippy `--all-targets -D warnings`
= 0 across domain+engine+mcp+cli** (the earlier commit-memory gap is closed via `-j1` + no-debuginfo).
⚠️ Found a latent compose index-hashing bug (`BUGS.md`); a visual eyeball of the new ops is still owed.
Remaining menu letters (owner's call): E brightness/contrast, F saturation/vibrance, H levels, I curves,
J white_balance, L gradient_map, N sharpen, O pixelate(standalone), P vignette, V bg-removal(AI),
W watermark, X device_frame.

### M2 — Viewport bridge spike `[DONE 2026-07-06]` ✅ decision de-risked
Built `crates/craws-app` (Tauri 2) + `app/` (React/WebGPU): window, 24MP sample, exposure slider,
pan/zoom, stats overlay, auto-bench. **All exit criteria met** (numbers in STATE + JOURNAL):
- pan/zoom (pure GPU) = **165 fps** ✅ (≥60 target); GPU compositing is free.
- authoritative full-frame slider = 7.8 fps (bridge = 104 ms / 9.4 MB) ❌ — confirms the bridge is
  the bottleneck, as anticipated.
- **optimistic in-shader preview = 165 fps, 0 bridge calls** ✅ (≥30 target) — pointwise ops
  (exposure/grayscale) applied in the fragment shader in linear light on a neutral base; engine
  reconciles on release. 21× the naive path, display-bound.
- Fallback ladder (mip degradation → native wgpu under transparent child webview → GPUI) NOT needed.
- Left for M3: changed-tiles-only streaming, mip base pyramid, tauri-specta typed IPC, and the
  bridge-throughput number matters only for non-shader ops (blur, resize-on-zoom).

### M3 — The editor `[TODO]`
- Viewport (from M2 spike) + properties panel + **linear chains UI** (ordered step list covers 80%
  of automation; full node-graph editor comes later — React Flow when it does).
- Beautiful from the start: design tokens, dark theme first, shadcn-family components.
- Promote the spike's ad-hoc pieces to real infra: tauri-specta typed commands, changed-tiles-only
  tile streaming + atlas (replace the single-texture full-frame upload), mip base pyramid, a
  generalized in-shader preview layer for all pointwise ops, `prefers-reduced-motion`-aware CSS/WAAPI
  micro-interactions.

## UI & ANIMATION PHILOSOPHY (owner contract)

Owner wants: **beautiful, animated, optimized**. All three, no trade-off accepted.
- **No framer-motion by default** — owner reports it feels laggy everywhere; every dependency must
  earn its frame budget. If a lib is ever reconsidered, it must win a measured A/B in WebView2, not vibes.
- CSS transitions + WAAPI, **compositor-friendly props only** (`transform`, `opacity`); never animate
  layout (width/height/top/left) on hot paths; `will-change` used sparingly and removed after.
- Micro-interactions ≤ 200ms, easing over springs for v1; respect `prefers-reduced-motion`.
- `[IDEA]` later: tiny custom animation utils (rAF/spring, ~1KB) if orchestration outgrows CSS/WAAPI.
- React perf hygiene: virtualized lists, memoized selectors (zustand), no re-render-driven animation,
  code-split panels.

## LATER HORIZONS `[IDEA]`

- Vision path: patch pooprusteek `server/openai.rs` + `*_compat.rs` to pass `image_url` parts through.
- Local pixel models in `craws-ai` via `ort` (ONNX): RMBG/BiRefNet (bg removal), Real-ESRGAN (upscale),
  SAM (selection), LaMa (inpainting) — download-once, offline after.
- Shared crates with pooprusteek (`provider/`, `mcp/`, `semantic/`) once both stabilize.
- Node-graph editor (React Flow), video, plugin system (wasm? — undecided).
