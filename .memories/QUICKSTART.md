# QUICKSTART
> 10-second orientation. Last updated: 2026-07-08 (26-tool MCP; 109 tests).

**What**: Craws — automation-first image editor. Rust engine (tiled, content-hash cached,
linear f32 premultiplied), pipelines as JSON, ports: CLI (live) / MCP (M1) / Tauri GUI (M2+).

**Where**: `crates/craws-{domain,engine,codecs,cli,mcp}`. Root `CLAUDE.md` bridges agents here.

**Run it**:
```sh
cargo run --release -p craws-cli -- ops                                  # list operations
cargo run --release -p craws-cli --example gen_sample -- sample.png     # make a 24MP test photo
cargo run --release -p craws-cli -- run pipeline.json --in sample.png --out out.webp
```

`pipeline.json`: `{ "version": 0, "steps": [ { "op": "resize", "width": 1920 }, { "op": "exposure", "stops": 0.4 } ] }`

**Verify (the gate for every change)**:
```sh
cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets
```
Expect: **109 tests** green, clippy **0 warnings**.

> ⚠️ This dev box has **no page file** (commit == RAM). Heavy parallel builds fail to mmap stdlib
> (`os error 1455`). Prefix cargo with `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUSTFLAGS="-C debuginfo=0"`
> and prefer per-crate (`-p craws-domain -p craws-engine -p craws-mcp -p craws-cli`); `craws-app` (Tauri)
> is heavy — rebuild only when you touch it.

**Benches** (record meaningful shifts in `JOURNAL/`):
```sh
cargo bench -p craws-engine --bench engine -- --quick
cargo bench -p craws-codecs --bench codecs -- --quick
```
