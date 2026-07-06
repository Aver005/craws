# MAP
> File → purpose. Last updated: 2026-07-06 (M0 tree).

```
Cargo.toml                      workspace: members, shared deps, release profile (thin LTO, cu=1)
CLAUDE.md                       agent bridge → .memories/ + hard invariants
README.md                       public pitch + usage

crates/craws-domain/            ZERO-I/O core
  src/lib.rs                    re-exports
  src/geometry.rs               Size, Rect (overflow-safe fits_in)
  src/pipeline.rs               Pipeline, OpSpec (serde `op`-tagged), Filter, validation → sizes

crates/craws-engine/            the executor
  src/lib.rs                    re-exports + engine invariants doc
  src/color.rs                  sRGB⇄linear (decode LUT; encode formula, clamp lives ONLY here)
  src/tile.rs                   Tile (exact-size), TileRef, TiledImage, grid math,
                                from/to sRGB8 (premultiply here), from/to flat f32, pixel()
  src/hash.rs                   ContentHash + Merkle derivation (src/pw/gl/glt domains)
  src/cache.rs                  TileCache: LRU by byte budget (HashMap + BTreeMap recency)
  src/ops.rs                    kernels: exposure, grayscale, crop (row-run gather), resize (imageops)
  src/engine.rs                 Engine::run — validate → per-step hash/cache/compute → RunStats
  benches/engine.rs             24MP: convert, exposure cold/cached, resize, chain cold/slider-warm

crates/craws-codecs/            format adapters (image crate: png/jpeg/webp)
  src/lib.rs                    decode (sniffing), encode (jpeg flattens over white), ImageFormat
  benches/codecs.rs             24MP decode/encode

crates/craws-cli/               port #1
  src/main.rs                   clap: `run` (timing report to stderr, --quiet, --quality), `ops`
  tests/e2e.rs                  drives the real binary: happy path, invalid pipeline, quiet, ops
  examples/gen_sample.rs        synthetic 24MP "photo" generator

crates/craws-mcp/               port #2 stub (M1: rmcp server)
  src/lib.rs                    placeholder const + plan doc

crates/craws-app/               port #3: Tauri 2 GUI shell (M2 spike)
  src/main.rs                   commands: load_source / render (raw binary frame) / report_bench;
                                20-byte frame header (magic,w,h,engine_us,convert_us) + sRGB8
  build.rs                      tauri_build
  tauri.conf.json               window, devUrl→dist, csp null (spike)
  capabilities/default.json     core:default only
  icons/icon.ico                placeholder (amber claw marks) — replace with real mascot
  Cargo.toml                    features: default=custom-protocol (embeds dist; tauri dev = no-default)

app/                            React front (Vite + TS + Tailwind 4 + WebGPU)
  src/main.tsx, styles.css      bootstrap
  src/ipc.ts                    typed bridge; parses raw frame header; makeRenderQueue (latest-wins)
  src/renderer.ts               Viewport: WebGPU quad, pan/zoom = GPU transform, in-shader
                                exposure/grayscale (srgb↔linear) for optimistic preview
  src/App.tsx                   overlay UI: slider, mode toggle (optimistic/authoritative),
                                stats, dual auto-bench
  index.html, package.json, vite.config.ts, tsconfig.json
```

## PERF NUMBERS (24MP, release, owner's machine — see STATE for the table)
M0 engine: warm slider-tweak 11.8 ms, cached op 85 µs, CLI end-to-end 556 ms.
M2 bridge spike: pan/zoom 165 fps; authoritative slider 7.8 fps (bridge 104 ms); optimistic 165 fps.
