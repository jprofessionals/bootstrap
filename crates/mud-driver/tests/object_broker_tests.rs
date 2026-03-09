use mud_driver::runtime::object_broker::ObjectBroker;
use mud_mop::message::{CachePolicy, Value};

#[test]
fn cache_miss_returns_none() {
    let broker = ObjectBroker::new();
    assert!(broker.check_cache(1, "get_name").is_none());
}

#[test]
fn cache_result_with_cacheable_then_hit() {
    let mut broker = ObjectBroker::new();

    broker.cache_result(
        42,
        "get_description",
        Value::String("A shiny sword".into()),
        CachePolicy::Cacheable,
    );

    let cached = broker
        .check_cache(42, "get_description")
        .expect("cacheable result should be in cache");
    assert_eq!(*cached, Value::String("A shiny sword".into()));
}

#[test]
fn volatile_results_are_not_cached() {
    let mut broker = ObjectBroker::new();

    broker.cache_result(
        42,
        "get_hp",
        Value::Int(100),
        CachePolicy::Volatile,
    );

    assert!(
        broker.check_cache(42, "get_hp").is_none(),
        "volatile result should not be stored in cache"
    );
}

#[test]
fn ttl_results_expire() {
    let mut broker = ObjectBroker::new();

    // Cache with a TTL of 0 seconds -- should expire immediately
    broker.cache_result(
        42,
        "get_weather",
        Value::String("sunny".into()),
        CachePolicy::Ttl(0),
    );

    // With a TTL of 0, the elapsed time (>= 0) should NOT be < 0, so it expires
    assert!(
        broker.check_cache(42, "get_weather").is_none(),
        "TTL(0) entry should be expired on next check"
    );
}

#[test]
fn ttl_results_available_before_expiry() {
    let mut broker = ObjectBroker::new();

    // Cache with a generous TTL
    broker.cache_result(
        42,
        "get_weather",
        Value::String("rainy".into()),
        CachePolicy::Ttl(3600),
    );

    let cached = broker
        .check_cache(42, "get_weather")
        .expect("TTL entry should be available before expiry");
    assert_eq!(*cached, Value::String("rainy".into()));
}

#[test]
fn invalidate_by_object_ids_clears_cache() {
    let mut broker = ObjectBroker::new();

    broker.cache_result(10, "get_name", Value::String("Alice".into()), CachePolicy::Cacheable);
    broker.cache_result(20, "get_name", Value::String("Bob".into()), CachePolicy::Cacheable);
    broker.cache_result(30, "get_name", Value::String("Charlie".into()), CachePolicy::Cacheable);

    broker.invalidate(&[10, 30]);

    assert!(broker.check_cache(10, "get_name").is_none(), "object 10 should be invalidated");
    assert!(broker.check_cache(30, "get_name").is_none(), "object 30 should be invalidated");

    let still_cached = broker
        .check_cache(20, "get_name")
        .expect("object 20 should still be cached");
    assert_eq!(*still_cached, Value::String("Bob".into()));
}

#[test]
fn invalidate_with_empty_list_is_noop() {
    let mut broker = ObjectBroker::new();
    broker.cache_result(10, "get_name", Value::String("Alice".into()), CachePolicy::Cacheable);

    broker.invalidate(&[]);

    assert!(broker.check_cache(10, "get_name").is_some(), "empty invalidation should be a no-op");
}

#[test]
fn cleanup_expired_entries() {
    let mut broker = ObjectBroker::new();

    // Cacheable entries should survive cleanup
    broker.cache_result(1, "get_name", Value::String("Alice".into()), CachePolicy::Cacheable);

    // TTL(0) entry should be cleaned up
    broker.cache_result(2, "get_weather", Value::String("sunny".into()), CachePolicy::Ttl(0));

    // TTL with generous time should survive
    broker.cache_result(3, "get_status", Value::String("online".into()), CachePolicy::Ttl(3600));

    broker.cleanup_expired();

    assert!(
        broker.check_cache(1, "get_name").is_some(),
        "cacheable entry should survive cleanup"
    );
    assert!(
        broker.check_cache(2, "get_weather").is_none(),
        "expired TTL entry should be cleaned up"
    );
    assert!(
        broker.check_cache(3, "get_status").is_some(),
        "non-expired TTL entry should survive cleanup"
    );
}

#[test]
fn multiple_methods_on_same_object_cached_independently() {
    let mut broker = ObjectBroker::new();

    broker.cache_result(42, "get_name", Value::String("Sword".into()), CachePolicy::Cacheable);
    broker.cache_result(42, "get_weight", Value::Int(5), CachePolicy::Cacheable);
    broker.cache_result(42, "get_value", Value::Int(100), CachePolicy::Cacheable);

    let name = broker.check_cache(42, "get_name").expect("get_name should be cached");
    let weight = broker.check_cache(42, "get_weight").expect("get_weight should be cached");
    let value = broker.check_cache(42, "get_value").expect("get_value should be cached");

    assert_eq!(*name, Value::String("Sword".into()));
    assert_eq!(*weight, Value::Int(5));
    assert_eq!(*value, Value::Int(100));

    // Caching a new result for one method should not affect others
    broker.cache_result(42, "get_name", Value::String("Axe".into()), CachePolicy::Cacheable);

    let updated_name = broker.check_cache(42, "get_name").unwrap();
    assert_eq!(*updated_name, Value::String("Axe".into()));

    let still_weight = broker.check_cache(42, "get_weight").unwrap();
    assert_eq!(*still_weight, Value::Int(5));
}

#[test]
fn different_objects_same_method_cached_independently() {
    let mut broker = ObjectBroker::new();

    broker.cache_result(1, "get_name", Value::String("Alice".into()), CachePolicy::Cacheable);
    broker.cache_result(2, "get_name", Value::String("Bob".into()), CachePolicy::Cacheable);

    assert_eq!(
        *broker.check_cache(1, "get_name").unwrap(),
        Value::String("Alice".into())
    );
    assert_eq!(
        *broker.check_cache(2, "get_name").unwrap(),
        Value::String("Bob".into())
    );
}
