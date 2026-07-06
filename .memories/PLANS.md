# PLANS
> Roadmap & active priorities. Last updated: 2026-07-06 (stack decision folded in; M2 redefined).

## MILESTONES

### M0 — Core without UI `[DONE 2026-07-06]`
Shipped as planned: workspace of 5 crates, tiled engine + Merkle content-hash cache, ops
(resize/crop/exposure/grayscale), png/jpeg/webp codecs, `craws run` CLI with timing report,
criterion benches. 43 tests, clippy 0. Numbers in `STATE.md` + `JOURNAL/2026-07-06.md`
(headline: warm slider tweak on 24MP chain = 11.8 ms; fully-cached op = 85 µs).
Deviations from plan: `Document` type deferred to M1+ (pipelines don't need it yet);
brightness shipped as photographic `exposure`; curves deferred.

### M1 — MCP demo with pooprusteek `[TODO]` ← NEXT
- `craws-mcp` on the official `rmcp` SDK: expose `open_image`, `resize`, `crop`, `apply_filter`,
  `export`, `describe_image` (stub until vision).
- Connect from pooprusteek via `/mcp add` (stdio). Demo: "take all screenshots in folder, crop,
  watermark, export webp" driven by the agent. This is the "it's alive" moment.

### M2 — Viewport bridge spike `[TODO]` (redefined: was "GPUI vs Tauri", stack is now decided)
Prove the numbers on the owner's machine before building the real UI:
- Tauri 2 app, React front; engine streams **changed tiles** (raw bytes, `tauri::ipc::Response` /
  `Channel`) → WebGPU canvas composites a tile atlas.
- Optimistic preview: exposure/curves applied as in-page WebGPU shader during slider drag (zero
  bridge traffic), engine recomputes authoritative result on release.
- Exit criteria: 24MP image, slider drag ≥ 30fps preview in WebView2, pan/zoom ≥ 60fps, numbers in JOURNAL.
- Fallback ladder if numbers fail: mip-level preview degradation → native wgpu surface under
  transparent child webview (unstable Tauri feature — last resort) → GPUI reopens.

### M3 — The editor `[TODO]`
- Viewport (from M2 spike) + properties panel + **linear chains UI** (ordered step list covers 80%
  of automation; full node-graph editor comes later — React Flow when it does).
- Beautiful from the start: design tokens, dark theme first, shadcn-family components.

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
