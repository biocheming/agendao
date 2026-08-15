use std::collections::BTreeMap;

struct CacheEntry<V> {
    value: V,
    bytes: usize,
    last_used: u64,
}

pub(crate) struct BoundedLruCache<K, V> {
    entries: BTreeMap<K, CacheEntry<V>>,
    bytes: usize,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
}

impl<K, V> BoundedLruCache<K, V>
where
    K: Clone + Ord,
    V: Clone,
{
    pub(crate) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            clock: 0,
            max_entries,
            max_bytes,
        }
    }

    pub(crate) fn get(&mut self, key: &K) -> Option<V> {
        self.clock = self.clock.saturating_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.clock;
        Some(entry.value.clone())
    }

    pub(crate) fn insert(&mut self, key: K, value: V, bytes: usize) -> usize {
        if self.max_entries == 0 || bytes > self.max_bytes {
            return 0;
        }

        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }

        let mut evictions = 0;
        while self.entries.len() >= self.max_entries
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.bytes);
                evictions += 1;
            }
        }

        self.clock = self.clock.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(
            key,
            CacheEntry {
                value,
                bytes,
                last_used: self.clock,
            },
        );
        evictions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn hit_refreshes_lru_and_eviction_is_deterministic() {
        let mut cache = BoundedLruCache::new(2, 16);
        let first: Arc<str> = Arc::from("first");
        cache.insert("a", first.clone(), first.len());
        cache.insert("b", Arc::from("second"), 6);

        let hit = cache.get(&"a").expect("cache hit");
        assert!(Arc::ptr_eq(&first, &hit));
        assert_eq!(cache.insert("c", Arc::from("third"), 5), 1);
        assert!(cache.get(&"b").is_none());
        assert!(cache.get(&"a").is_some());
        assert!(cache.get(&"c").is_some());
    }

    #[test]
    fn replacement_updates_bytes_and_oversized_values_are_not_cached() {
        let mut cache = BoundedLruCache::new(2, 8);
        cache.insert("same", Arc::<str>::from("123456"), 6);
        assert_eq!(cache.insert("same", Arc::from("1"), 1), 0);
        assert_eq!(cache.insert("other", Arc::from("1234567"), 7), 0);
        assert_eq!(cache.get(&"same").as_deref(), Some("1"));
        assert_eq!(cache.get(&"other").as_deref(), Some("1234567"));

        cache.insert("too-large", Arc::from("123456789"), 9);
        assert!(cache.get(&"too-large").is_none());
    }
}
