//! In-memory tile cache: content-hash → tile, LRU-evicted under a byte budget.
//!
//! Correctness never depends on the cache — evicting everything only costs
//! recomputation. Hand-rolled (HashMap + BTreeMap recency index) to keep the
//! dependency tree lean; swap for something smarter when profiles say so.

use crate::hash::ContentHash;
use crate::tile::Tile;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

struct Entry {
    tile: Arc<Tile>,
    tick: u64,
    bytes: u64,
}

pub struct TileCache {
    map: HashMap<ContentHash, Entry>,
    /// recency index: tick → hash (ticks are unique)
    recency: BTreeMap<u64, ContentHash>,
    next_tick: u64,
    bytes: u64,
    budget: u64,
}

impl TileCache {
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            map: HashMap::new(),
            recency: BTreeMap::new(),
            next_tick: 0,
            bytes: 0,
            budget: budget_bytes,
        }
    }

    pub fn get(&mut self, hash: &ContentHash) -> Option<Arc<Tile>> {
        let tick = self.bump();
        let e = self.map.get_mut(hash)?;
        self.recency.remove(&e.tick);
        e.tick = tick;
        self.recency.insert(tick, *hash);
        Some(Arc::clone(&e.tile))
    }

    pub fn insert(&mut self, hash: ContentHash, tile: Arc<Tile>) {
        let tick = self.bump();
        if let Some(e) = self.map.get_mut(&hash) {
            // same content, refresh recency only
            self.recency.remove(&e.tick);
            e.tick = tick;
            self.recency.insert(tick, hash);
            return;
        }
        let bytes = tile.bytes();
        self.map.insert(hash, Entry { tile, tick, bytes });
        self.recency.insert(tick, hash);
        self.bytes += bytes;
        self.evict_to_budget();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    fn bump(&mut self) -> u64 {
        let t = self.next_tick;
        self.next_tick += 1;
        t
    }

    fn evict_to_budget(&mut self) {
        while self.bytes > self.budget {
            // oldest tick first; the just-touched entry has the max tick, so it
            // survives unless it alone exceeds the budget
            let Some((&tick, &hash)) = self.recency.iter().next() else { break };
            self.recency.remove(&tick);
            if let Some(e) = self.map.remove(&hash) {
                self.bytes -= e.bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::digest_bytes;

    fn tile_of_bytes(n: u32) -> Arc<Tile> {
        // n pixels ⇒ 16n bytes
        Arc::new(Tile::solid(n, 1, [0.0, 0.0, 0.0, 1.0]))
    }

    #[test]
    fn hit_and_miss() {
        let mut c = TileCache::new(u64::MAX);
        let h = digest_bytes(b"a");
        assert!(c.get(&h).is_none());
        c.insert(h, tile_of_bytes(1));
        assert!(c.get(&h).is_some());
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn duplicate_insert_does_not_double_count() {
        let mut c = TileCache::new(u64::MAX);
        let h = digest_bytes(b"a");
        c.insert(h, tile_of_bytes(4));
        let bytes = c.bytes();
        c.insert(h, tile_of_bytes(4));
        assert_eq!(c.bytes(), bytes);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn evicts_least_recently_used_under_budget() {
        // budget fits exactly two 16-byte tiles
        let mut c = TileCache::new(32);
        let (a, b, d) = (digest_bytes(b"a"), digest_bytes(b"b"), digest_bytes(b"d"));
        c.insert(a, tile_of_bytes(1));
        c.insert(b, tile_of_bytes(1));
        assert!(c.get(&a).is_some(), "touch a → b becomes LRU");
        c.insert(d, tile_of_bytes(1));
        assert!(c.get(&b).is_none(), "b evicted");
        assert!(c.get(&a).is_some());
        assert!(c.get(&d).is_some());
        assert!(c.bytes() <= 32);
    }
}
