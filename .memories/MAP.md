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
```
