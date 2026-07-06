# STATE
> Live project snapshot. Update on every meaningful change.
> Last updated: 2026-07-06 — **M0 shipped**: workspace, tiled engine with content-hash cache, codecs, CLI runner. 43 tests, clippy 0. Next: M1 (MCP).

## SNAPSHOT

- **M0 core is live.** `craws run pipeline.json --in a.png --out b.webp` works end-to-end:
  24MP through a 4-step pipeline in ~556 ms total (see numbers below).
- The BLAZING contract has its first proof: warm slider tweak on a 24MP chain = **11.8 ms**
  (resize stays cached, only downstream pointwise ops recompute); fully-cached 24MP op = **85 µs**.
- Crates: `craws-domain` (types+validation), `craws-engine` (tiles/hash/cache/ops/runner),
  `craws-codecs` (png/jpeg/webp via `image`), `craws-cli` (bin `craws`), `craws-mcp` (stub).
- UI stack `[DECIDED]`: Tauri 2 + React; animations CSS/WAAPI-only, framer-motion rejected.
- Deferred on purpose: `craws-ops`/`craws-gpu`/`craws-ai`/`craws-project` split out only when the
  boundaries start to hurt — ops live inside the engine for now (see ARCHITECTURE).

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
