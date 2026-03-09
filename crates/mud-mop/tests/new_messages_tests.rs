use std::collections::HashMap;

use mud_core::types::AreaId;
use mud_mop::message::{AdapterMessage, CachePolicy, DriverMessage, Value};

#[test]
fn reload_program_with_files_list() {
    let msg = DriverMessage::ReloadProgram {
        area_id: AreaId::new("game", "dungeon"),
        path: "/world/game/dungeon".into(),
        files: vec![
            "monster.lpc".into(),
            "treasure.lpc".into(),
            "trap.lpc".into(),
        ],
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let DriverMessage::ReloadProgram {
        area_id,
        path,
        files,
    } = &decoded
    {
        assert_eq!(area_id, &AreaId::new("game", "dungeon"));
        assert_eq!(path, "/world/game/dungeon");
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], "monster.lpc");
        assert_eq!(files[1], "treasure.lpc");
        assert_eq!(files[2], "trap.lpc");
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn reload_program_with_empty_files() {
    let msg = DriverMessage::ReloadProgram {
        area_id: AreaId::new("system", "core"),
        path: "/world/system/core".into(),
        files: vec![],
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: DriverMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);
}

#[test]
fn program_reloaded_with_version() {
    let msg = AdapterMessage::ProgramReloaded {
        area_id: AreaId::new("game", "tavern"),
        path: "/world/game/tavern/bartender.lpc".into(),
        version: 7,
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let AdapterMessage::ProgramReloaded {
        area_id,
        path,
        version,
    } = &decoded
    {
        assert_eq!(area_id, &AreaId::new("game", "tavern"));
        assert_eq!(path, "/world/game/tavern/bartender.lpc");
        assert_eq!(*version, 7);
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn program_reload_error_with_error_string() {
    let msg = AdapterMessage::ProgramReloadError {
        area_id: AreaId::new("game", "dungeon"),
        path: "/world/game/dungeon/broken.lpc".into(),
        error: "syntax error at line 42: unexpected '}'".into(),
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let AdapterMessage::ProgramReloadError {
        area_id,
        path,
        error,
    } = &decoded
    {
        assert_eq!(area_id, &AreaId::new("game", "dungeon"));
        assert_eq!(path, "/world/game/dungeon/broken.lpc");
        assert!(error.contains("syntax error"));
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn invalidate_cache_with_object_ids() {
    let msg = AdapterMessage::InvalidateCache {
        object_ids: vec![100, 200, 300],
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let AdapterMessage::InvalidateCache { object_ids } = &decoded {
        assert_eq!(object_ids, &vec![100, 200, 300]);
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn invalidate_cache_with_empty_ids() {
    let msg = AdapterMessage::InvalidateCache {
        object_ids: vec![],
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);
}

#[test]
fn call_result_with_cacheable_policy() {
    let msg = AdapterMessage::CallResult {
        request_id: 1,
        result: Value::String("Goblin".into()),
        cache: Some(CachePolicy::Cacheable),
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let AdapterMessage::CallResult {
        request_id,
        result,
        cache,
    } = &decoded
    {
        assert_eq!(*request_id, 1);
        assert_eq!(*result, Value::String("Goblin".into()));
        assert_eq!(*cache, Some(CachePolicy::Cacheable));
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn call_result_with_volatile_policy() {
    let msg = AdapterMessage::CallResult {
        request_id: 2,
        result: Value::Int(42),
        cache: Some(CachePolicy::Volatile),
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let AdapterMessage::CallResult { cache, .. } = &decoded {
        assert_eq!(*cache, Some(CachePolicy::Volatile));
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn call_result_with_ttl_policy() {
    let msg = AdapterMessage::CallResult {
        request_id: 3,
        result: Value::Map(HashMap::from([
            ("weather".into(), Value::String("sunny".into())),
            ("temp".into(), Value::Int(72)),
        ])),
        cache: Some(CachePolicy::Ttl(60)),
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let AdapterMessage::CallResult { cache, .. } = &decoded {
        assert_eq!(*cache, Some(CachePolicy::Ttl(60)));
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn call_result_without_cache_field_backward_compat() {
    let msg = AdapterMessage::CallResult {
        request_id: 4,
        result: Value::Bool(true),
        cache: None,
    };
    let bytes = rmp_serde::to_vec_named(&msg).expect("serialize");
    let decoded: AdapterMessage = rmp_serde::from_slice(&bytes).expect("deserialize");
    assert_eq!(msg, decoded);

    if let AdapterMessage::CallResult { cache, .. } = &decoded {
        assert_eq!(*cache, None);
    } else {
        panic!("decoded to wrong variant");
    }
}

#[test]
fn cache_policy_serialization_round_trip() {
    let policies = vec![
        CachePolicy::Volatile,
        CachePolicy::Cacheable,
        CachePolicy::Ttl(0),
        CachePolicy::Ttl(60),
        CachePolicy::Ttl(3600),
        CachePolicy::Ttl(u64::MAX),
    ];

    for policy in policies {
        let bytes = rmp_serde::to_vec_named(&policy).expect("serialize CachePolicy");
        let decoded: CachePolicy =
            rmp_serde::from_slice(&bytes).expect("deserialize CachePolicy");
        assert_eq!(
            policy, decoded,
            "round-trip failed for {:?}",
            policy
        );
    }
}

#[test]
fn cache_policy_debug_format() {
    // Verify Debug is derived and produces meaningful output
    let volatile = format!("{:?}", CachePolicy::Volatile);
    assert!(volatile.contains("Volatile"));

    let cacheable = format!("{:?}", CachePolicy::Cacheable);
    assert!(cacheable.contains("Cacheable"));

    let ttl = format!("{:?}", CachePolicy::Ttl(60));
    assert!(ttl.contains("Ttl"));
    assert!(ttl.contains("60"));
}

#[test]
fn cache_policy_clone_and_eq() {
    let original = CachePolicy::Ttl(120);
    let cloned = original.clone();
    assert_eq!(original, cloned);

    assert_ne!(CachePolicy::Volatile, CachePolicy::Cacheable);
    assert_ne!(CachePolicy::Ttl(60), CachePolicy::Ttl(120));
    assert_eq!(CachePolicy::Ttl(60), CachePolicy::Ttl(60));
}
