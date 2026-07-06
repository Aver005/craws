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

crates/craws-mcp/               port #2: MCP server (rmcp 2.1, stdio, protocol 2024-11-05)
  src/session.rs                Session — engine-facing core, NO mcp types (unit-testable):
                                open_bytes/open_path, apply(OpSpec)→new handle, info, export; immutable
                                image handles ("img-N"), shared Engine cache, SessionError
  src/lib.rs                    rmcp wrapper: Craws { session, tool_router }, #[tool_router]/#[tool]
                                (open_image/resize/crop/exposure/grayscale/image_info/export),
                                #[tool_handler(router = self.tool_router)], get_info (instructions);
                                results are JSON text (image parts get dropped by clients)
  src/main.rs                   bin `craws-mcp`: serve(stdio()); logs to STDERR only (stdout=protocol)
  tests/stdio_protocol.rs       spawns the real binary, full JSON-RPC handshake + batch + error paths
                                (mirrors pooprusteek's client) — doubles as the demo transcript

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
