//! Uploaded assets kept by key and dropped by last use.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// What an asset holds on the host and the GPU together.
pub trait Charged {
    fn bytes(&self) -> u64;
}

struct Slot<V> {
    value: Arc<V>,
    bytes: u64,
    /// The clock when the entry was last inserted or looked up.
    used: AtomicU64,
}

/// Assets keyed by the caller. An insert or a lookup stamps its entry with a clock, and [`Self::end_frame`] drops
/// the entries not stamped since the frame began, least recently stamped first, until the idle ones fit a byte
/// budget. What a frame draws is never dropped, whatever the budget.
pub struct AssetCache<V> {
    map: HashMap<String, Slot<V>>,
    clock: AtomicU64,
    /// The clock when the current frame began: an entry stamped at or after it is in use.
    frame: u64,
    bytes: u64,
}

impl<V> Default for AssetCache<V> {
    fn default() -> Self {
        Self { map: HashMap::new(), clock: AtomicU64::new(1), frame: 1, bytes: 0 }
    }
}

impl<V: Charged> AssetCache<V> {
    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed)
    }

    /// The asset under `key`, stamped as used this frame.
    pub fn get(&self, key: &str) -> Option<&Arc<V>> {
        let slot = self.map.get(key)?;
        slot.used.store(self.tick(), Ordering::Relaxed);
        Some(&slot.value)
    }

    /// The asset under `key`, without stamping it: for bookkeeping, not for drawing.
    pub fn peek(&self, key: &str) -> Option<&Arc<V>> {
        self.map.get(key).map(|s| &s.value)
    }

    /// Whether `key` holds an asset, without stamping it.
    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Keeps `value` under `key`, stamped as used this frame; the asset it replaces comes back.
    pub fn insert(&mut self, key: String, value: Arc<V>) -> Option<Arc<V>> {
        let bytes = value.bytes();
        self.bytes += bytes;
        let slot = Slot { value, bytes, used: AtomicU64::new(self.tick()) };
        self.map.insert(key, slot).map(|old| self.uncharge(old))
    }

    pub fn remove(&mut self, key: &str) -> Option<Arc<V>> {
        self.map.remove(key).map(|old| self.uncharge(old))
    }

    /// Keeps the assets `keep` accepts.
    pub fn retain(&mut self, mut keep: impl FnMut(&String, &Arc<V>) -> bool) {
        let bytes = &mut self.bytes;
        self.map.retain(|k, slot| {
            let kept = keep(k, &slot.value);
            if !kept {
                *bytes -= slot.bytes;
            }
            kept
        });
    }

    fn uncharge(&mut self, old: Slot<V>) -> Arc<V> {
        self.bytes -= old.bytes;
        old.value
    }

    /// Every key and asset, without stamping them.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Arc<V>)> {
        self.map.iter().map(|(k, s)| (k, &s.value))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Bytes of every asset kept.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Ends a frame: drops the assets not used since the frame began, least recently used first, until the ones
    /// left idle hold at most `budget` bytes. Returns how many were dropped. The next frame begins.
    pub fn end_frame(&mut self, budget: u64) -> usize {
        let frame = self.frame;
        let mut idle: Vec<(u64, u64, &String)> = self
            .map
            .iter()
            .map(|(k, s)| (s.used.load(Ordering::Relaxed), s.bytes, k))
            .filter(|&(used, _, _)| used < frame)
            .collect();
        let mut idle_bytes: u64 = idle.iter().map(|&(_, b, _)| b).sum();
        idle.sort_unstable();
        let mut drop = Vec::new();
        for (_, bytes, key) in idle {
            if idle_bytes <= budget {
                break;
            }
            idle_bytes -= bytes;
            drop.push(key.clone());
        }
        for key in &drop {
            self.remove(key);
        }
        self.frame = self.tick();
        drop.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetCache, Charged};
    use std::sync::Arc;

    struct Blob(u64);

    impl Charged for Blob {
        fn bytes(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn idle_entries_beyond_the_budget_go_least_recently_used_first() {
        let mut c = AssetCache::<Blob>::default();
        for k in ["a", "b", "c"] {
            c.insert(k.to_string(), Arc::new(Blob(10)));
        }
        assert_eq!((c.len(), c.bytes()), (3, 30));
        // inserted this frame: all three are in use, whatever the budget
        assert_eq!(c.end_frame(0), 0);
        assert_eq!(c.len(), 3);
        // a is looked up, so b and c are idle: two idle entries fit twenty bytes
        assert!(c.get("a").is_some());
        assert_eq!(c.end_frame(20), 0);
        // d is inserted and c looked up this frame, so a and b are idle: ten bytes hold one of them, and b was
        // used before a
        c.insert("d".to_string(), Arc::new(Blob(10)));
        assert!(c.get("c").is_some());
        assert_eq!(c.end_frame(10), 1);
        assert!(!c.contains_key("b"), "b was last used before a");
        assert!(c.contains_key("a") && c.contains_key("c") && c.contains_key("d"));
        assert_eq!(c.bytes(), 30);
        // nothing is used this frame and nothing idle may stay
        assert_eq!(c.end_frame(0), 3);
        assert!(c.is_empty());
        assert_eq!(c.bytes(), 0);
    }

    #[test]
    fn a_lookup_in_the_frame_an_entry_is_dropped_in_keeps_it() {
        let mut c = AssetCache::<Blob>::default();
        c.insert("a".to_string(), Arc::new(Blob(10)));
        c.end_frame(u64::MAX);
        c.end_frame(u64::MAX);
        assert!(c.get("a").is_some());
        assert_eq!(c.end_frame(0), 0, "looked up this frame");
        assert!(c.peek("a").is_some());
        assert_eq!(c.end_frame(0), 1, "a peek is not a use");
    }

    #[test]
    fn removals_and_replacements_keep_the_byte_count() {
        let mut c = AssetCache::<Blob>::default();
        c.insert("a".to_string(), Arc::new(Blob(10)));
        c.insert("b".to_string(), Arc::new(Blob(20)));
        c.insert("c".to_string(), Arc::new(Blob(30)));
        assert!(c.insert("a".to_string(), Arc::new(Blob(5))).is_some(), "the old value comes back");
        assert_eq!(c.bytes(), 55);
        assert!(c.remove("b").is_some());
        assert!(c.remove("b").is_none());
        assert_eq!(c.bytes(), 35);
        c.retain(|k, v| k != "c" && v.bytes() > 0);
        assert_eq!((c.len(), c.bytes()), (1, 5));
        assert_eq!(c.peek("a").map(|v| v.bytes()), Some(5));
        assert!(c.peek("b").is_none());
    }
}
