# STATE
> Live project snapshot. Update on every meaningful change.
> Last updated: 2026-07-06 — **M0 + M2 viewport-bridge spike shipped**. Tauri 2 + React validated
> with numbers: optimistic in-shader preview = 165 fps (display-bound), 0 bridge calls. Next: M1 (MCP).

## SNAPSHOT

- **M0 core is live.** `craws run pipeline.json --in a.png --out b.webp` end-to-end: 24MP / 4-step
  pipeline in ~556 ms. Warm slider tweak on a 24MP chain = **11.8 ms**; fully-cached op = **85 µs**.
- **M2 spike is live** (`crates/craws-app` Tauri 2 + `app/` React/WebGPU): real window, 24MP sample,
  exposure slider, pan/zoom, live stats overlay, auto-bench. **The stack decision is now proven by
  measurement, not argument** (see M2 SPIKE VERDICT below).
- Crates: `craws-domain` (types+validation), `craws-engine` (tiles/hash/cache/ops/runner),
  `craws-codecs` (png/jpeg/webp via `image`), `craws-cli` (bin `craws`), `craws-mcp` (stub),
  `craws-app` (Tauri 2 shell: `load_source`/`render`/`report_bench`, raw binary IPC).
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
| CLI end-to-end: PNG decode → 4-step pipeline → WebP write | 556 ms |
| ingest sRGB8 → linear f32 tiles | 45–64 ms |
| export tiles → sRGB8 | ~106 ms |
| exposure over 24MP, cold / **cached** | 49 ms / **85 µs** |
| resize 6000→1920 Lanczos3, cold | ~303 ms |
| chain (resize+exposure+crop+gray), cold / **slider-tweak warm** | ~335 ms / **11.8 ms (~85 fps)** |
| decode jpeg / png (24MP) | 128 ms / 157 ms |
| encode jpeg q90 | **893 ms** ⚠️ |
| encode png | 182 ms |

`[BUG]`-grade perf note: jpeg **encode** (image crate, single-threaded) is ~7× slower than decode —
first BLAZING candidate (mozjpeg / jpeg-encoder / turbojpeg behind the same codecs API).

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
| `cargo test --workspace` | **43 passing** (7 domain + 26 engine + 5 codecs + 1 cli + 4 e2e) |
| `cargo clippy --workspace --all-targets` | **0 warnings** |
| Benches | `cargo bench -p craws-engine --bench engine` / `-p craws-codecs --bench codecs` |
| CI | `[TODO]` (mirror pooprusteek's build+test win/linux) |

## CURRENT FOCUS

1. **M1 — MCP demo**: `craws-mcp` on `rmcp`, tools `open_image`/`resize`/`crop`/`apply_filter`/
   `export`; connect from pooprusteek via `/mcp add`; watermark-batch demo scenario.
2. Then M2 — viewport bridge spike (Tauri, exit numbers in `PLANS.md`).
