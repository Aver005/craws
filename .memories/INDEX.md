# .memories INDEX
> Agent entry point. If you were handed this project cold, START HERE. Last updated: 2026-07-06
> (M0 shipped: tiled engine + content-hash cache + CLI runner; 43 tests, clippy 0; slider-tweak on
> a warm 24MP chain = 11.8 ms. Next: M1 MCP server).

> `CLAUDE.md` at the repo root is the auto-loaded bridge into this folder for Claude Code.
> Other agents must be told to read `.memories/INDEX.md` first.

## 0. THE BLAZING CONTRACT (prime directive)

Owner's standing order: **wherever performance can be squeezed out — squeeze it.** This is not a
polish-phase item; it shapes every design decision from day one. Concretely:

- **Engine**: tiles + content-hash cache, linear f32/f16 premultiplied internals, zune codecs, rayon
  for CPU, wgpu compute for heavy kernels, undo as graph-edit journal (not pixel snapshots).
- **Bridge (Rust ↔ WebView)**: pixels NEVER travel as JSON. Raw binary IPC (`tauri::ipc::Response`,
  `Channel`), changed-tiles-only streaming at the right mip level, authoritative compute in the engine
  with optimistic preview shaders in-page.
- **UI**: compositor-friendly animations only (`transform`/`opacity`), no layout thrash, virtualized
  lists, measured frame budgets. Beautiful AND fast is the requirement, not a trade-off.
- **Proof over vibes**: benchmarks (criterion) land with the first engine crate; perf claims in these
  docs get numbers, not adjectives.

## 1. READ ORDER

### Core (read top-to-bottom for a full mental model)
| Step | File | Why |
|------|------|-----|
| 1 | `QUICKSTART.md` | 10s orientation — what/where/how to run and verify |
| 2 | `STATE.md` | Current snapshot — done / measured / next |
| 3 | `MAP.md` | File → purpose map of the tree |
| 4 | `PLANS.md` | Milestones M0–M3, UI & animation philosophy, later horizons |
| 5 | `ARCHITECTURE.md` | Workspace crates, engine principles, Tauri shell design |
| 6 | `CONVENTIONS.md` | Code style + discipline to follow when editing |
| 7 | `JOURNAL/` | Dated log of sessions and decisions |

### Appear later (create when there is content, not before)
`GLOSSARY.md` · `BUGS.md` · `LEARNINGS.md` · `reference/` (deep dives: ENGINE, BRIDGE, MCP…)

## 2. KEY SIGNALS

| Signal | Meaning |
|--------|---------|
| `[DONE]` | Implemented and working |
| `[WIP]` | In progress, partial |
| `[TODO]` | Planned, not started |
| `[DECIDED]` | Decision fixed — do not relitigate without new evidence |
| `[BUG]` | Known defect |
| `[IDEA]` | Proposed, not committed |
| `[?]` | Needs investigation |
| `→ path:line` | Cross-reference to source |

## 3. EXTERNAL CONTEXT

- **What Craws is**: open-source image editor (photos now, maybe video later) for *developers* —
  automation-first: pipelines/chains of parametrized operations, headless CLI, MCP server. Rust engine.
  Name = claws (Rust crab) + draws.
- **Sibling project**: `E:\Projects\Me\pooprusteek` — owner's TUI AI agent (~35k LOC Rust). Its
  `.memories/` is the template for this folder. Integration plan: Craws exposes an MCP server
  (`craws-mcp`, rmcp crate) → pooprusteek's mature MCP client drives it; Craws borrows LLM access via
  pooprusteek's OpenAI-compatible `--serve` endpoint (note: it currently strips `image_url` parts —
  vision needs a patch there).
- **UI stack** `[DECIDED 2026-07-06]`: Tauri 2 + React. GPUI demoted to "return if the product pivots
  to real-time painting". Rationale + verification trail: `JOURNAL/2026-07-06.md`.

## 4. MAINTENANCE RULE

Update the relevant file on every meaningful change and bump its `Last updated`. Add a
`JOURNAL/{date}.md` entry for notable sessions. Keep claims tied to `→ file:line` once code exists.
If a fact here contradicts the code, the **code wins** — fix the memory.
