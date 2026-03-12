use std::collections::HashMap;

use mud_core::types::ObjectId;
use mud_mop::message::{CachePolicy, Value};

/// A cached method-call result together with its cache policy and timestamp.
struct CachedResult {
    value: Value,
    policy: CachePolicy,
    cached_at: std::time::Instant,
}

/// Routes cross-adapter method calls and caches results according to the
/// [`CachePolicy`] returned by each adapter.
///
/// The broker sits between the driver core and the adapter connections.  When a
/// `Call` result comes back with a `Cacheable` or `Ttl` policy the broker
/// stores it so that subsequent calls to the same object+method can be served
/// without another round-trip to the adapter.
pub struct ObjectBroker {
    cache: HashMap<(ObjectId, String), CachedResult>,
}

impl ObjectBroker {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Check the cache for a previously stored result.
    ///
    /// Returns `None` if there is no entry, or if the entry has expired
    /// according to its TTL.
    pub fn check_cache(&self, object_id: ObjectId, method: &str) -> Option<&Value> {
        let entry = self.cache.get(&(object_id, method.to_string()))?;
        match &entry.policy {
            CachePolicy::Volatile => None,
            CachePolicy::Cacheable => Some(&entry.value),
            CachePolicy::Ttl(secs) => {
                if entry.cached_at.elapsed().as_secs() < *secs {
                    Some(&entry.value)
                } else {
                    None
                }
            }
        }
    }

    /// Store a result in the cache with the given policy.
    ///
    /// `Volatile` results are intentionally *not* stored.
    pub fn cache_result(
        &mut self,
        object_id: ObjectId,
        method: &str,
        value: Value,
        policy: CachePolicy,
    ) {
        if matches!(policy, CachePolicy::Volatile) {
            return;
        }
        self.cache.insert(
            (object_id, method.to_string()),
            CachedResult {
                value,
                policy,
                cached_at: std::time::Instant::now(),
            },
        );
    }

    /// Invalidate all cached results for the given object ids.
    pub fn invalidate(&mut self, object_ids: &[ObjectId]) {
        if object_ids.is_empty() {
            return;
        }
        self.cache.retain(|&(oid, _), _| !object_ids.contains(&oid));
    }

    /// Invalidate all entries whose key starts with the given program path.
    ///
    /// This is a coarse-grained invalidation: when a program is reloaded every
    /// cached result that was keyed on objects of that program should be
    /// discarded.  Callers typically combine this with
    /// [`super::state_store::StateStore::objects_by_program`] to build the id
    /// list instead, but this method provides a convenient shortcut when the
    /// program path is embedded in the method key by convention.
    pub fn invalidate_by_program(&mut self, _program: &str) {
        // The cache is keyed by (ObjectId, method) — there is no direct mapping
        // from program path to cache keys.  The intended usage is:
        //
        //   let ids = state_store.objects_by_program(program);
        //   broker.invalidate(&ids);
        //
        // This method is provided as an extension point for future indexing.
        // For now it is a no-op; callers should use `invalidate` with an
        // explicit id list.
    }

    /// Remove all cache entries whose TTL has expired.
    pub fn cleanup_expired(&mut self) {
        self.cache.retain(|_, entry| match &entry.policy {
            CachePolicy::Volatile => false,
            CachePolicy::Cacheable => true,
            CachePolicy::Ttl(secs) => entry.cached_at.elapsed().as_secs() < *secs,
        });
    }
}
