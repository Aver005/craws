# MAP
> File → purpose. Last updated: 2026-07-08 (26-tool MCP: +geometry/color/filter/compare/meta; +filter.rs).

```
Cargo.toml                      workspace: members, shared deps, release profile (thin LTO, cu=1)
CLAUDE.md                       agent bridge → .memories/ + hard invariants
README.md                       public pitch + usage

crates/craws-domain/            ZERO-I/O core
  src/lib.rs                    re-exports
  src/color.rs                  Rgba8 (straight-alpha sRGB; alpha defaults to 255 in JSON)
  src/geometry.rs               Size, Rect (overflow-safe fits_in)
  src/pipeline.rs               Pipeline + 18 OpSpec variants (serde `op`-tagged): resize/crop/rotate/
                                flip/pad/exposure/grayscale/hue_rotate/invert/blur/redact/spotlight/
                                beautify/draw_rect/ellipse/line/arrow/text. Enums Filter, AlignX/AlignY,
                                FlipAxis, RedactMode (type-tagged pixelate|blur|fill). validation→sizes;
                                rotated_size() (90° exact / bbox on expand). Errors incl. ResultTooLarge.
                                NOTE: OpSpec has a non-Copy field (DrawText.text:String) → output_size
                                uses `match *self` with `..` on that arm.

crates/craws-engine/            the executor
  src/lib.rs                    re-exports + engine invariants doc
  src/color.rs                  sRGB⇄linear: decode 256-LUT; encode **16-bit LUT** (65536 entries, no
                                powf on the hot export path — exact for u8 roundtrip; clamp lives ONLY
                                here) + float linear_to_srgb_f32 / srgb_to_linear_f32 (used by invert)
  src/tile.rs                   Tile (exact-size), TileRef, TiledImage, grid math,
                                from/to sRGB8 (premultiply here), from/to flat f32, pixel()
  src/hash.rs                   ContentHash + Merkle derivation (src/pw/gl/glt/cmp domains);
                                compose_signature (params ⊕ input tile hashes) for multi-input ops
  src/cache.rs                  TileCache: LRU by byte budget (HashMap + BTreeMap recency)
  src/ops.rs                    kernels: exposure, grayscale, hue_rotate (+hue_matrix, SVG luma-preserving),
                                invert (perceptual sRGB), crop (row-run gather), resize (→resample),
                                rotate (90° exact permute / bilinear inverse-map + sampler), flip,
                                pad (solid canvas + row-run copy), content_bounds + trim (tight
                                non-bg bbox → crop, content-derived hashes)
  src/resample.rs               own separable resampler: Contribs (precomputed per-output weights,
                                filter-scaled for downsampling), horizontal (streams rows from tiles,
                                no full flat src copy) + vertical passes, both rayon-parallel.
                                nearest/bilinear/catmull_rom/lanczos3. Engine has NO `image` dep.
  src/draw.rs                   SDF vector rasterizer: rect (rounded, fill+stroke), ellipse, line,
                                arrow (shaft+V-head), spotlight (dim outside a rounded/feathered window,
                                whole-image); 1px AA coverage composited in LINEAR light; only bbox tiles
                                recompute. Paint = linear premul. pub composite_mask (→text); pub
                                clone_all; sd_round_rect/aa are pub(crate) (reused by filter::beautify).
  src/filter.rs                 neighborhood ops: gaussian_blur_flat (used by blur_region) + blur1
                                (1-channel, for the shadow) + blur (whole image, tile-row BANDED/
                                STREAMING — streams source rows via copy_row_into, writes tiles direct,
                                NO full-image flat); redact (pixelate/blur/fill a region, only rect
                                tiles recompute) via write_region; regions read tile-direct via
                                read_region (row-run gather, no pixel()); opaque fill skips the source
                                read; beautify (rounded corners + 1-channel soft drop-shadow + padded
                                background, BeautifyParams). Premultiplied linear.
  src/text.rs                   text layout (ab_glyph: kern, \n, align_x/y) → glyph coverage mask →
                                draw::composite_mask. TextParams built from OpSpec::DrawText.
  src/fonts.rs                  font loading: embedded default (assets/CascadiaCode.ttf, OFL) +
                                load-by-explicit-path, cached by path. NO fontdb (determinism).
  src/compose.rs                multi-input ops (outside the single-input Pipeline): overlay (paste
                                image at x,y + opacity) + collage (justified-rows layout by aspect,
                                each full row fills width) + diff (DiffView difference/heatmap/
                                side_by_side + DiffStats fraction/max) → called directly by the MCP
                                session. Content-addressed: private blend() builds pixels, then stamp()
                                re-hashes via hash::compose_signature (fixed the old index-hash bug).
                                blend reads `top` via one to_flat_f32 then indexes flat (no per-pixel pixel()).
  assets/CascadiaCode.ttf       embedded default font (Microsoft, SIL OFL 1.1)
  assets/CascadiaCode-LICENSE.txt  the font's OFL license (must ship with the font)
  src/engine.rs                 Engine::run — validate → per-step hash/cache/compute → RunStats
  benches/engine.rs             24MP: convert, exposure cold/cached, resize, chain cold/slider-warm

crates/craws-codecs/            format adapters (image: png/jpeg-decode/webp; jpeg-encoder: jpeg-encode)
  src/lib.rs                    decode (sniffing, image), encode: png/webp via image, jpeg via
                                jpeg-encoder (SIMD, flattens over white, 16-bit dim guard), ImageFormat
  benches/codecs.rs             24MP decode/encode

crates/craws-cli/               port #1
  src/main.rs                   clap: `run` (timing report to stderr, --quiet, --quality), `ops`
                                (OPS_HELP now lists ALL pipeline OpSpecs incl. geometry/color/filter;
                                r##"…"## raw string — hex colors contain `"#`). diff/run_pipeline are
                                NOT here (two-input / meta = MCP-only).
  tests/e2e.rs                  drives the real binary: happy path, invalid pipeline, quiet, ops
  examples/gen_sample.rs        synthetic 24MP "photo" generator

skills/craws-mcp/               Agent Skill: how to drive the craws MCP (SKILL.md + references/
  SKILL.md                      mental model (immutable handles → chain image_id), coordinate
                                sourcing (Playwright boundingBox + DPR), annotation/composition
                                recipes, pitfalls, worked example
  references/tools.md           exhaustive per-tool reference (params, defaults, returns, errors)
  references/cookbook.md        Playwright→craws→Outline pipeline, redaction, before/after, batch
  (validated: with-skill A/B prevents the stale-handle bug the baseline fell into)

crates/craws-mcp/               port #2: MCP server (rmcp 2.1, stdio, protocol 2024-11-05)
  src/session.rs                Session — engine-facing core, NO mcp types (unit-testable):
                                open_bytes/open_path, apply(OpSpec)→new handle, info, export; +trim
                                (content-dependent size), +diff (→DiffResult{image,fraction,max}),
                                +run_pipeline (whole Pipeline in one call). immutable "img-N" handles,
                                shared Engine cache, SessionError (+NothingToTrim, +SizeMismatch)
  src/lib.rs                    rmcp wrapper: Craws { session, tool_router }, #[tool_router]/#[tool]
                                (26 tools: I/O open/info/export; transform resize/crop/rotate/flip/pad/
                                exposure/grayscale/hue_rotate/invert; filter blur/redact/spotlight/
                                beautify; annotation draw_rect/ellipse/line/arrow/text; multi overlay/
                                collage/trim/diff/run_pipeline). parse_color/_align_x/_y/_axis/
                                _redact_mode/_diff_view/_pipeline; #[tool_handler], get_info; results
                                are JSON text (diff also returns fraction_changed/max_difference)
  src/fonts.rs                  port-side font-NAME→path resolution via fontdb (system fonts);
                                keeps the engine deterministic (engine only sees explicit paths)
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
Engine: warm slider-tweak 11.8 ms, cached op 85 µs, CLI end-to-end 336 ms (was 556).
Resize 6000→1920 lanczos3: 54 ms (was 323, own resampler). jpeg encode q90: 375 ms (was 910, SIMD).
M2 bridge spike: pan/zoom 165 fps; authoritative slider 7.8 fps (bridge 104 ms); optimistic 165 fps.
