# Craws 🦀🎨

> claws + draws — a blazing-fast, automation-first image editor for developers. Written in Rust.

Craws treats image editing as **pipelines**: chains of parametrized operations you can run from a
GUI, a headless CLI, CI, or drive from an AI agent over MCP. The engine is tiled, content-hash
cached, and works in linear-light f32 — tweak one parameter and only the affected tiles of the
affected steps recompute.

**Status: pre-alpha (M0).** Engine core + CLI pipeline runner. No GUI yet (Tauri 2 + React planned).

## Try it

```sh
cargo run --release -p craws-cli -- run pipeline.json --in photo.jpg --out result.webp
```

`pipeline.json`:

```json
{
  "version": 0,
  "steps": [
    { "op": "resize", "width": 1600 },
    { "op": "exposure", "stops": 0.5 },
    { "op": "crop", "x": 100, "y": 50, "width": 1280, "height": 720 }
  ]
}
```

`cargo run -p craws-cli -- ops` lists available operations.

## Workspace

| Crate | Role |
|---|---|
| `craws-domain` | Pure types: geometry, pipeline spec, validation. Zero I/O. |
| `craws-engine` | Tiled executor: content-hash cache, linear f32 premultiplied color, rayon. |
| `craws-codecs` | Format adapters (png / jpeg / webp) |
| `craws-cli` | Headless pipeline runner (`craws`) |
| `craws-mcp` | MCP server for AI agents (stub — M1) |

License: MIT
