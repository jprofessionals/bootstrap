use lpc_vm::bytecode::{CompiledFunction, CompiledProgram, LpcValue, ObjectRef};
use lpc_vm::object::{DependencyGraph, ObjectError, ObjectTable};

/// Create a minimal compiled program for testing purposes.
fn make_program(path: &str, inherits: &[&str], globals: u16) -> CompiledProgram {
    CompiledProgram {
        path: path.to_string(),
        version: 0,
        inherits: inherits.iter().map(|s| s.to_string()).collect(),
        functions: vec![],
        global_count: globals,
        global_names: (0..globals).map(|i| format!("g{}", i)).collect(),
    }
}

/// Create a program with a named function.
fn make_program_with_fn(
    path: &str,
    inherits: &[&str],
    func_name: &str,
    modifiers: &[u8],
) -> CompiledProgram {
    CompiledProgram {
        path: path.to_string(),
        version: 0,
        inherits: inherits.iter().map(|s| s.to_string()).collect(),
        functions: vec![CompiledFunction {
            name: func_name.to_string(),
            arity: 0,
            varargs: false,
            local_count: 0,
            code: vec![],
            modifiers: modifiers.to_vec(),
        }],
        global_count: 0,
        global_names: vec![],
    }
}

// =========================================================================
// Register master object
// =========================================================================

#[test]
fn register_master_object() {
    let mut table = ObjectTable::new();
    let program = make_program("/std/room", &[], 2);
    let obj = table.register_master(program);
    assert_eq!(obj.path, "/std/room");
    assert_eq!(obj.id, 0);
    assert!(!obj.is_lightweight);
}

#[test]
fn get_master_after_register() {
    let mut table = ObjectTable::new();
    let program = make_program("/std/room", &[], 2);
    table.register_master(program);
    let master = table.get_master("/std/room");
    assert!(master.is_some());
    assert_eq!(master.unwrap().path, "/std/room");
}

// =========================================================================
// Clone object
// =========================================================================

#[test]
fn clone_object() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 2));
    let clone = table.clone_object("/std/room").unwrap();
    assert!(clone.id > 0);
    assert_eq!(clone.path, "/std/room");
    assert!(!clone.is_lightweight);
}

#[test]
fn clone_nonexistent_fails() {
    let mut table = ObjectTable::new();
    let result = table.clone_object("/nonexistent");
    assert!(result.is_err());
    match result.unwrap_err() {
        ObjectError::NotFound(p) => assert_eq!(p, "/nonexistent"),
        other => panic!("expected NotFound, got: {:?}", other),
    }
}

// =========================================================================
// Lightweight object
// =========================================================================

#[test]
fn create_lightweight_object() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/lib/data", &[], 1));
    let lwo = table.new_lightweight("/lib/data").unwrap();
    assert!(lwo.id > 0);
    assert!(lwo.is_lightweight);
    assert_eq!(lwo.path, "/lib/data");
}

// =========================================================================
// Destruct object
// =========================================================================

#[test]
fn destruct_clone() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let clone = table.clone_object("/std/room").unwrap();
    let clone_id = clone.id;
    table.destruct(&clone).unwrap();
    // Clone should no longer be findable
    assert!(table.find_by_id(clone_id).is_none());
}

#[test]
fn destruct_master_removes_clones() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let clone = table.clone_object("/std/room").unwrap();
    let clone_id = clone.id;
    let master_ref = ObjectRef {
        id: 0,
        path: "/std/room".to_string(),
        is_lightweight: false,
    };
    table.destruct(&master_ref).unwrap();
    assert!(table.get_master("/std/room").is_none());
    assert!(table.find_by_id(clone_id).is_none());
}

// =========================================================================
// Find object
// =========================================================================

#[test]
fn find_object_by_path() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let found = table.find_object("/std/room");
    assert!(found.is_some());
    assert_eq!(found.unwrap().path, "/std/room");
}

#[test]
fn find_object_by_clone_path() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let clone = table.clone_object("/std/room").unwrap();
    let search = format!("/std/room#{}", clone.id);
    let found = table.find_object(&search);
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, clone.id);
}

#[test]
fn find_nonexistent_object() {
    let table = ObjectTable::new();
    assert!(table.find_object("/nowhere").is_none());
}

// =========================================================================
// Object name
// =========================================================================

#[test]
fn object_name_master() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let master_ref = ObjectRef {
        id: 0,
        path: "/std/room".to_string(),
        is_lightweight: false,
    };
    assert_eq!(table.object_name(&master_ref), "/std/room");
}

#[test]
fn object_name_clone() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let clone = table.clone_object("/std/room").unwrap();
    let name = table.object_name(&clone);
    assert_eq!(name, format!("/std/room#{}", clone.id));
}

#[test]
fn object_name_lightweight() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/lib/data", &[], 0));
    let lwo = table.new_lightweight("/lib/data").unwrap();
    let name = table.object_name(&lwo);
    assert_eq!(name, "/lib/data#-1");
}

// =========================================================================
// Global variable get/set
// =========================================================================

#[test]
fn global_get_set() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 2));
    let master_ref = ObjectRef {
        id: 0,
        path: "/std/room".to_string(),
        is_lightweight: false,
    };
    table.set_global(&master_ref, 0, LpcValue::Int(42)).unwrap();
    let val = table.get_global(&master_ref, 0).unwrap();
    assert_eq!(val, &LpcValue::Int(42));
}

#[test]
fn global_default_is_nil() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 2));
    let master_ref = ObjectRef {
        id: 0,
        path: "/std/room".to_string(),
        is_lightweight: false,
    };
    let val = table.get_global(&master_ref, 0).unwrap();
    assert_eq!(val, &LpcValue::Nil);
}

#[test]
fn clone_global_get_set() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 2));
    let clone = table.clone_object("/std/room").unwrap();
    table
        .set_global(&clone, 0, LpcValue::String("hello".into()))
        .unwrap();
    let val = table.get_global(&clone, 0).unwrap();
    assert_eq!(val, &LpcValue::String("hello".to_string()));
}

// =========================================================================
// Dependency graph
// =========================================================================

#[test]
fn dependency_graph_add_and_query() {
    let mut graph = DependencyGraph::new();
    graph.add_dependency("/area/room", "/std/room");
    let deps = graph.get_dependents("/std/room");
    assert_eq!(deps, vec!["/area/room".to_string()]);
}

#[test]
fn dependency_graph_no_dependents() {
    let graph = DependencyGraph::new();
    let deps = graph.get_dependents("/std/room");
    assert!(deps.is_empty());
}

#[test]
fn walk_transitive_dependents() {
    let mut graph = DependencyGraph::new();
    graph.add_dependency("/b", "/a");
    graph.add_dependency("/c", "/b");
    graph.add_dependency("/d", "/c");
    let deps = graph.walk_dependents("/a");
    // Should include /b, /c, /d (in some order)
    assert!(deps.contains(&"/b".to_string()));
    assert!(deps.contains(&"/c".to_string()));
    assert!(deps.contains(&"/d".to_string()));
}

// =========================================================================
// Upgrade program
// =========================================================================

#[test]
fn upgrade_program_bumps_version() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 2));
    assert_eq!(table.get_master("/std/room").unwrap().version, 1);

    let new_program = make_program("/std/room", &[], 3);
    table.upgrade_program("/std/room", new_program).unwrap();
    assert_eq!(table.get_master("/std/room").unwrap().version, 2);
}

#[test]
fn upgrade_program_returns_dependents() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    table.register_master(make_program("/area/castle", &["/std/room"], 0));

    let new_program = make_program("/std/room", &[], 0);
    let deps = table.upgrade_program("/std/room", new_program).unwrap();
    assert!(deps.contains(&"/area/castle".to_string()));
}

#[test]
fn upgrade_resizes_globals() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 2));
    let clone = table.clone_object("/std/room").unwrap();

    // Upgrade with more globals
    let new_program = make_program("/std/room", &[], 5);
    table.upgrade_program("/std/room", new_program).unwrap();

    // Should be able to access new global slots
    table.set_global(&clone, 4, LpcValue::Int(99)).unwrap();
    let val = table.get_global(&clone, 4).unwrap();
    assert_eq!(val, &LpcValue::Int(99));
}

// =========================================================================
// Inheritance resolution
// =========================================================================

#[test]
fn resolve_function_own_program() {
    let mut table = ObjectTable::new();
    table.register_master(make_program_with_fn("/std/room", &[], "create", &[]));
    let result = table.resolve_function("/std/room", "create");
    assert!(result.is_some());
    let (path, idx) = result.unwrap();
    assert_eq!(path, "/std/room");
    assert_eq!(idx, 0);
}

#[test]
fn resolve_function_through_inheritance() {
    let mut table = ObjectTable::new();
    table.register_master(make_program_with_fn("/std/room", &[], "create", &[]));
    table.register_master(make_program("/area/castle", &["/std/room"], 0));
    let result = table.resolve_function("/area/castle", "create");
    assert!(result.is_some());
    let (path, _) = result.unwrap();
    assert_eq!(path, "/std/room");
}

#[test]
fn resolve_function_private_hidden_from_child() {
    let mut table = ObjectTable::new();
    // MOD_PRIVATE = 0x01
    table.register_master(make_program_with_fn("/std/room", &[], "secret", &[0x01]));
    table.register_master(make_program("/area/castle", &["/std/room"], 0));
    let result = table.resolve_function("/area/castle", "secret");
    // Private function should not be visible from child
    assert!(result.is_none());
}

#[test]
fn resolve_function_not_found() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let result = table.resolve_function("/std/room", "nonexistent");
    assert!(result.is_none());
}

#[test]
fn inherits_from_check() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/object", &[], 0));
    table.register_master(make_program("/std/room", &["/std/object"], 0));
    table.register_master(make_program("/area/castle", &["/std/room"], 0));
    assert!(table.inherits_from("/area/castle", "/std/room"));
    assert!(table.inherits_from("/area/castle", "/std/object"));
    assert!(!table.inherits_from("/std/room", "/area/castle"));
}

// =========================================================================
// is_master
// =========================================================================

#[test]
fn is_master_true() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let obj = ObjectRef {
        id: 0,
        path: "/std/room".to_string(),
        is_lightweight: false,
    };
    assert!(table.is_master(&obj));
}

#[test]
fn is_master_false_for_clone() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 0));
    let clone = table.clone_object("/std/room").unwrap();
    assert!(!table.is_master(&clone));
}

// =========================================================================
// get_program follows clone to master
// =========================================================================

#[test]
fn get_program_for_clone() {
    let mut table = ObjectTable::new();
    table.register_master(make_program("/std/room", &[], 2));
    let clone = table.clone_object("/std/room").unwrap();
    let program = table.get_program(&clone).unwrap();
    assert_eq!(program.path, "/std/room");
}
