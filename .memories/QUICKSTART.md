# QUICKSTART
> 10-second orientation. Last updated: 2026-07-06 (M0).

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
Expect: 43 tests green, clippy **0 warnings**.

**Benches** (record meaningful shifts in `JOURNAL/`):
```sh
cargo bench -p craws-engine --bench engine -- --quick
cargo bench -p craws-codecs --bench codecs -- --quick
```
