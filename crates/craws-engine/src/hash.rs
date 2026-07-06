//! Merkle-style content addressing.
//!
//! The whole point: **pixel data is hashed at most once** (the source bytes).
//! Every downstream tile's identity is derived from op parameters + input tile
//! identities, so cache keys for an entire interactive session cost microseconds.
//!
//! Derivation domains are tagged to keep the spaces disjoint:
//! - source tile:      `H("craws/src" ‖ source_digest ‖ tile_index)`
//! - pointwise tile:   `H("craws/pw"  ‖ op_digest ‖ input_tile_hash)`
//! - global signature: `H("craws/gl"  ‖ op_digest ‖ input_tile_hashes…)`
//! - global out tile:  `H("craws/glt" ‖ signature ‖ tile_index)`

use craws_domain::OpSpec;

/// 256-bit content identity (blake3).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 32]);

impl std::fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // first 8 bytes are plenty for logs
        for b in &self.0[..8] {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Digest raw source bytes (file contents, network body, …).
/// The single place where bulk data gets hashed; blake3 chews GB/s.
pub fn digest_bytes(bytes: &[u8]) -> ContentHash {
    ContentHash(*blake3::hash(bytes).as_bytes())
}

/// Canonical digest of an op's parameters. Uses the serde JSON form: struct
/// field order is fixed at compile time, so the encoding is deterministic.
pub fn op_digest(spec: &OpSpec) -> ContentHash {
    let bytes = serde_json::to_vec(spec).expect("OpSpec serialization is infallible");
    ContentHash(*blake3::hash(&bytes).as_bytes())
}

pub fn source_tile_hash(source: ContentHash, tile_index: u32) -> ContentHash {
    derive(b"craws/src", &[&source.0, &tile_index.to_le_bytes()])
}

pub fn pointwise_tile_hash(op: ContentHash, input: ContentHash) -> ContentHash {
    derive(b"craws/pw", &[&op.0, &input.0])
}

pub fn global_signature<'a>(
    op: ContentHash,
    inputs: impl IntoIterator<Item = &'a ContentHash>,
) -> ContentHash {
    let mut h = blake3::Hasher::new();
    h.update(b"craws/gl");
    h.update(&op.0);
    for i in inputs {
        h.update(&i.0);
    }
    ContentHash(*h.finalize().as_bytes())
}

pub fn global_tile_hash(signature: ContentHash, tile_index: u32) -> ContentHash {
    derive(b"craws/glt", &[&signature.0, &tile_index.to_le_bytes()])
}

fn derive(tag: &[u8], parts: &[&[u8]]) -> ContentHash {
    let mut h = blake3::Hasher::new();
    h.update(tag);
    for p in parts {
        h.update(p);
    }
    ContentHash(*h.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use craws_domain::{Filter, OpSpec};

    #[test]
    fn op_digest_is_stable_and_param_sensitive() {
        let a = OpSpec::Exposure { stops: 0.5 };
        let b = OpSpec::Exposure { stops: 0.5 };
        let c = OpSpec::Exposure { stops: 0.6 };
        assert_eq!(op_digest(&a), op_digest(&b));
        assert_ne!(op_digest(&a), op_digest(&c));
    }

    #[test]
    fn domains_are_disjoint() {
        let x = digest_bytes(b"x");
        // same material through different derivations must not collide
        assert_ne!(source_tile_hash(x, 0), pointwise_tile_hash(x, x));
        assert_ne!(source_tile_hash(x, 0), source_tile_hash(x, 1));
        assert_ne!(global_tile_hash(x, 0), source_tile_hash(x, 0));
    }

    #[test]
    fn different_ops_same_params_differ() {
        // Grayscale has no params; compare against exposure via signature path
        let g = op_digest(&OpSpec::Grayscale);
        let r = op_digest(&OpSpec::Resize { width: Some(1), height: None, filter: Filter::default() });
        assert_ne!(g, r);
    }
}
