# CLAUDE.md

**Start every session by reading `.memories/INDEX.md`** — it is the project knowledge base
(state, plans, architecture blueprint, journal). Update it on every meaningful change; if a fact
there contradicts the code, the code wins — fix the memory.

## What this is

Craws — open-source, automation-first image editor for developers. Rust engine (tiles +
content-hash cache, linear f32 premultiplied color), headless CLI, MCP server; Tauri 2 + React
shell later. Full architecture: `.memories/ARCHITECTURE.md`.

## Prime directive: the BLAZING contract

Wherever performance can be squeezed out — squeeze it (see `.memories/INDEX.md §0`).
Pixels never travel as JSON. Perf claims get numbers (criterion), not adjectives.

## Rules

- Dependency arrows point inward only: `domain ← engine`; codecs/gpu/ai are adapters;
  cli/mcp/app are equal ports over the engine and never know each other.
- Verification gate (all must pass before a change is "done"):
  `cargo build` · `cargo test` · `cargo clippy --all-targets` with **0 warnings**.
- Internal pixel format is linear-light premultiplied RGBA f32 — never store sRGB inside the engine.
- Don't clamp linear values mid-pipeline (HDR headroom); clamp only at u8 export.
