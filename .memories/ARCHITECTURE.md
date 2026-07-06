# ARCHITECTURE
> Blueprint + what's real. M0 core is implemented (domain/engine/codecs/cli + mcp stub);
> gpu/ai/project crates and the Tauri shell are still TARGET. Last updated: 2026-07-06 (M0).

## WORKSPACE LAYOUT (target)

```
craws/
├─ Cargo.toml              # [workspace]
├─ crates/
│  ├─ craws-domain/        # Document, Layer, Node, Pipeline/Graph, params, color types — ZERO I/O deps
│  ├─ craws-ops/           # operations (crop, resize, curves, blend…) — pure functions + registry
│  │                       #   (lives inside craws-engine until the boundary hurts)
│  ├─ craws-engine/        # executor: planning, tile scheduler, content-hash cache, undo/redo — THE façade
│  ├─ craws-codecs/        # format adapters (zune-image family, image, jxl-oxide…) behind a Codec trait
│  ├─ craws-gpu/           # wgpu compute kernels + tile render helpers (engine adapter)
│  ├─ craws-ai/            # ModelProvider trait + ort (ONNX) impls; LLM = HTTP client to pooprusteek
│  ├─ craws-project/       # .craws file serialization (DTOs separate from domain, versioned)
│  ├─ craws-cli/           # headless runner            ┐
│  ├─ craws-mcp/           # MCP server (rmcp)          ├─ EQUAL PORTS over craws-engine
│  └─ craws-app/           # Tauri 2 shell (Rust side)  ┘
└─ app/                    # React front (Vite, TS, Tailwind, shadcn-family, zustand, tauri-specta bindings)
```

**The one dependency rule**: arrows point inward only (`domain ← ops ← engine`); codecs/gpu/ai are
adapters behind engine traits; cli/mcp/app know the engine, never each other. Start with ~5 crates
(M0), split when boundaries are proven by compilation pain, not upfront.

**M0 reality (2026-07-06)**: 5 crates live — `domain`, `engine` (ops inside → `src/ops.rs`),
`codecs`, `cli` (bin `craws`), `mcp` (stub). Codecs and engine don't know each other at all:
codecs produce plain sRGB RGBA8, ports hand buffers over (`TiledImage::from_srgb_rgba8`).
`ops`/`gpu`/`ai`/`project` crates and `app/` split out later.

## ENGINE PRINCIPLES (the BLAZING core — non-negotiable, designed in from day one)

- **Tiles**: image = grid of 256²/512² tiles; every node's output cached per-tile by
  **content-hash(inputs + params)**. A slider tweak recomputes only affected tiles of affected nodes.
- **Color**: internal format = linear-light f32 (f16 where it wins) RGBA, **premultiplied**; ICC/sRGB
  conversion at boundaries only. Retrofitting this later is misery — never ship v0 without it.
- **Parallelism**: rayon for CPU tile batches; wgpu compute for heavy kernels (blur, resample, curves
  on big selections). CPU path always exists (headless CI runs without GPU).
- **Undo/redo**: journal of graph edits (cheap, unbounded), NOT pixel snapshots. Pixel deltas per tile
  only for future destructive brush tools.
- **Codecs**: zune family first (fastest Rust decoders), `image` crate as fallback breadth.
- **Determinism**: same pipeline + same inputs ⇒ bit-identical outputs (this is what makes headless
  CI/CD usage trustworthy — it's the product's spine, not a nice-to-have).

## SHELL ARCHITECTURE (Tauri 2 + React) `[DECIDED 2026-07-06]`

Verified against current Tauri 2 docs (2026-07-06): IPC is custom-protocol-based (HTTP-like
performance), raw binary payloads via `tauri::ipc::Response`, chunked streaming via `tauri::ipc::Channel`.

- **Pixels never ride JSON.** Tile bytes go through raw IPC / custom protocol; commands and params go
  through tauri-specta-typed commands (TS types generated from Rust — one contract, zero drift).
- **Viewport = WebGPU canvas in the page** holding a tile-atlas texture cache. Engine pushes only
  *changed* tiles at the mip level matching current zoom.
- **Optimistic preview shaders**: per-pixel ops (exposure, curves, WB) run as in-page WebGPU shaders
  on already-resident textures during drag → zero bridge traffic; engine computes the authoritative
  result on release and reconciles. Makes photo-editing UX feel native.
- **Known ceiling** `[DECIDED]`: low-latency 120Hz brush painting is out of v1 scope. Escape hatches,
  in order: mip-degraded preview → native wgpu surface under transparent child webview (Tauri
  `unstable` feature; no official example exists — verified 2026-07-06) → reopen GPUI.
- Memory budget honesty: WebView2 baseline ~200–400MB — accepted cost of the decision.

## PORTS

`craws-cli` (headless pipelines: `craws run pipeline.json`), `craws-mcp` (rmcp server for agents —
pooprusteek first), `craws-app` (Tauri GUI). All three are thin translations onto `craws-engine`'s
API; a feature that can't be expressed through the engine façade is a design smell.
