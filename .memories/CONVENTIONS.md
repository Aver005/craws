# CONVENTIONS
> Code style to follow when editing. Last updated: 2026-07-06.

- **Gate**: `cargo build && cargo test && cargo clippy --all-targets` — clippy stays at **0 warnings**
  (pooprusteek discipline). Tests live inline (`#[cfg(test)] mod tests`) next to the code;
  cross-process tests in `crates/craws-cli/tests/`.
- **Perf claims get numbers.** A change that touches a hot path runs the relevant bench before/after;
  meaningful shifts land in `JOURNAL/{date}.md`. No adjectives without measurements.
- **Dependency arrows point inward.** `domain` depends on serde+thiserror only. Ports never import
  each other. New heavy deps need a reason the BLAZING contract accepts.
- **Color discipline**: sRGB exists only at the boundaries (`color.rs`, codecs). Inside: linear
  premultiplied f32; no clamping mid-pipeline — `linear_to_srgb8` is the single clamp point.
- **Hash discipline**: every new op must define its cache derivation (pointwise vs global) in
  `hash.rs` terms; derivation domains are tagged strings (`craws/…`) — never reuse a tag.
- **Git (owner's hard rule)**: agents NEVER `git commit` or `git push` — the owner does both himself.
  Preparing the tree and suggesting a commit message as text is fine. No `Co-Authored-By` /
  "Generated with" trailers in commit messages, ever.
- **Errors**: `thiserror` in libraries, `anyhow` + context strings only in ports (CLI).
- **Docs**: file-top `//!` says what the module is *for* and which invariants it owns; comments
  state constraints, not narration. English everywhere in code and `.memories/`.
