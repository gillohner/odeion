//! Answer caches with a size limit.
//!
//! A cache whose keys come from requests (search text, profile keys, tag labels) grows by one
//! entry per new request; entries past their time were passed over when read but never dropped,
//! so such a cache grew for as long as the node ran.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Keep `value` under `key`, dated now. A new key in a full cache first makes room
/// ([`make_room`]); a key already there is replaced in place and drops nothing.
pub(crate) fn insert<K: Eq + Hash, V>(
    cache: &mut HashMap<K, (Instant, V)>,
    key: K,
    value: V,
    max: usize,
    ttl: Duration,
) {
    if !cache.contains_key(&key) {
        make_room(cache, max, ttl);
    }
    cache.insert(key, (Instant::now(), value));
}

/// Before an entry is added to a full cache: the ones past `ttl` go, and when it is still full
/// of fresh ones, the older half.
pub(crate) fn make_room<K: Eq + Hash, V>(cache: &mut HashMap<K, (Instant, V)>, max: usize, ttl: Duration) {
    if cache.len() < max {
        return;
    }
    cache.retain(|_, (at, _)| at.elapsed() < ttl);
    if cache.len() < max {
        return;
    }
    let mut ages: Vec<Instant> = cache.values().map(|(at, _)| *at).collect();
    ages.sort_unstable();
    let cut = ages[ages.len() / 2];
    cache.retain(|_, (at, _)| *at > cut);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache has a size: entries past their time go when it is full, and when it is full of
    /// fresh ones the older half goes, so the newest are always kept.
    #[test]
    fn a_cache_does_not_grow_without_end() {
        // An entry made at `made` is past this time when it is looked at; one dated later is not
        let ttl = Duration::from_nanos(1);
        let made = Instant::now();
        let fresh = |n: u32| made + Duration::from_secs(3600 + u64::from(n));
        let mut cache: HashMap<u32, (Instant, ())> = HashMap::new();
        // Below the limit nothing goes, however old
        for n in 0..9 {
            cache.insert(n, (made, ()));
        }
        make_room(&mut cache, 10, ttl);
        assert_eq!(cache.len(), 9);
        // At the limit: the ones past their time
        cache.insert(9, (fresh(9), ()));
        make_room(&mut cache, 10, ttl);
        assert_eq!(cache.keys().copied().collect::<Vec<_>>(), [9]);
        // Full of fresh ones: the older half
        for n in 0..10 {
            cache.insert(n, (fresh(n), ()));
        }
        make_room(&mut cache, 10, ttl);
        let mut left: Vec<u32> = cache.keys().copied().collect();
        left.sort_unstable();
        assert_eq!(left, [6, 7, 8, 9]);
        // As requests come in, one per key: never more than the limit, the newest always there
        let mut cache: HashMap<u32, (Instant, ())> = HashMap::new();
        for n in 0..5000 {
            insert(&mut cache, n, (), 512, Duration::from_secs(300));
            assert!(cache.len() <= 512);
        }
        assert!(cache.contains_key(&4999));
    }

    /// Refreshing a key that is already cached replaces it and drops nothing else, even when the
    /// cache is full of fresh entries.
    #[test]
    fn a_key_already_there_drops_nothing() {
        let ttl = Duration::from_secs(300);
        let mut cache: HashMap<u32, (Instant, u32)> = HashMap::new();
        for n in 0..10 {
            insert(&mut cache, n, n, 10, ttl);
        }
        assert_eq!(cache.len(), 10);
        insert(&mut cache, 3, 33, 10, ttl);
        assert_eq!(cache.len(), 10);
        assert_eq!(cache[&3].1, 33);
        // A new key makes room
        insert(&mut cache, 10, 10, 10, ttl);
        assert!(cache.len() <= 10 && cache.contains_key(&10));
    }
}
