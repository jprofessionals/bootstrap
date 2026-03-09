use mud_driver::runtime::version_tree::VersionTree;

#[test]
fn register_program_and_get_info() {
    let mut tree = VersionTree::new();
    tree.register("/world/npc.lpc", "lpc", vec![]);

    let info = tree.get("/world/npc.lpc").expect("program should be registered");
    assert_eq!(info.path, "/world/npc.lpc");
    assert_eq!(info.language, "lpc");
    assert_eq!(info.version, 1);
    assert!(info.dependencies.is_empty());
}

#[test]
fn version_starts_at_1() {
    let mut tree = VersionTree::new();
    tree.register("/world/base.lpc", "lpc", vec![]);

    let version = tree.version_of("/world/base.lpc").expect("should have version");
    assert_eq!(version, 1);
}

#[test]
fn bump_version_returns_incremented_value() {
    let mut tree = VersionTree::new();
    tree.register("/world/npc.lpc", "lpc", vec![]);

    let v2 = tree.bump_version("/world/npc.lpc").expect("bump should succeed");
    assert_eq!(v2, 2);

    let v3 = tree.bump_version("/world/npc.lpc").expect("bump should succeed");
    assert_eq!(v3, 3);

    // Verify via get as well
    assert_eq!(tree.version_of("/world/npc.lpc"), Some(3));
}

#[test]
fn bump_version_unregistered_returns_none() {
    let mut tree = VersionTree::new();
    assert!(tree.bump_version("/world/nonexistent.lpc").is_none());
}

#[test]
fn add_dependency_and_get_dependents() {
    let mut tree = VersionTree::new();
    tree.register("/world/base.lpc", "lpc", vec![]);
    tree.register(
        "/world/npc.lpc",
        "lpc",
        vec!["/world/base.lpc".to_string()],
    );

    let dependents = tree.get_dependents("/world/base.lpc");
    assert_eq!(dependents, vec!["/world/npc.lpc"]);
}

#[test]
fn get_dependents_returns_empty_for_no_dependents() {
    let mut tree = VersionTree::new();
    tree.register("/world/leaf.lpc", "lpc", vec![]);

    assert!(tree.get_dependents("/world/leaf.lpc").is_empty());
}

#[test]
fn walk_transitive_dependents_linear_chain() {
    // A -> B -> C: walk from A should return B and C
    let mut tree = VersionTree::new();
    tree.register("/a.lpc", "lpc", vec![]);
    tree.register("/b.lpc", "lpc", vec!["/a.lpc".to_string()]);
    tree.register("/c.lpc", "lpc", vec!["/b.lpc".to_string()]);

    let mut result = tree.walk_dependents("/a.lpc");
    result.sort();

    let mut expected = vec!["/b.lpc".to_string(), "/c.lpc".to_string()];
    expected.sort();

    assert_eq!(result, expected);
}

#[test]
fn walk_transitive_dependents_does_not_include_self() {
    let mut tree = VersionTree::new();
    tree.register("/a.lpc", "lpc", vec![]);
    tree.register("/b.lpc", "lpc", vec!["/a.lpc".to_string()]);

    let result = tree.walk_dependents("/a.lpc");
    assert!(!result.contains(&"/a.lpc".to_string()), "self should not be in walk result");
}

#[test]
fn unregister_removes_edges() {
    let mut tree = VersionTree::new();
    tree.register("/base.lpc", "lpc", vec![]);
    tree.register("/child.lpc", "lpc", vec!["/base.lpc".to_string()]);

    // Before unregister
    assert_eq!(tree.get_dependents("/base.lpc").len(), 1);

    tree.unregister("/child.lpc");

    // After unregister
    assert!(tree.get("/child.lpc").is_none());
    assert!(
        tree.get_dependents("/base.lpc").is_empty(),
        "reverse edge should be removed when child is unregistered"
    );
}

#[test]
fn programs_by_language_filter() {
    let mut tree = VersionTree::new();
    tree.register("/world/npc.lpc", "lpc", vec![]);
    tree.register("/world/portal.rb", "ruby", vec![]);
    tree.register("/world/handler.java", "jvm", vec![]);
    tree.register("/world/monster.lpc", "lpc", vec![]);

    let mut lpc_programs = tree.programs_by_language("lpc");
    lpc_programs.sort();

    let mut expected = vec!["/world/monster.lpc", "/world/npc.lpc"];
    expected.sort();

    assert_eq!(lpc_programs, expected);

    let ruby_programs = tree.programs_by_language("ruby");
    assert_eq!(ruby_programs, vec!["/world/portal.rb"]);

    let python_programs = tree.programs_by_language("python");
    assert!(python_programs.is_empty());
}

#[test]
fn diamond_dependency() {
    // D depends on B and C; B depends on A; C depends on A
    // Walk from D should return nothing (D has no dependents)
    // Walk from A should return B, C, and any dependents of B and C (which is D)
    let mut tree = VersionTree::new();
    tree.register("/a.lpc", "lpc", vec![]);
    tree.register(
        "/b.lpc",
        "lpc",
        vec!["/a.lpc".to_string()],
    );
    tree.register(
        "/c.lpc",
        "lpc",
        vec!["/a.lpc".to_string()],
    );
    tree.register(
        "/d.lpc",
        "lpc",
        vec!["/b.lpc".to_string(), "/c.lpc".to_string()],
    );

    // Walk from D: D has no dependents
    let from_d = tree.walk_dependents("/d.lpc");
    assert!(from_d.is_empty(), "D has no dependents");

    // Walk from A: should find B, C, and D
    let mut from_a = tree.walk_dependents("/a.lpc");
    from_a.sort();

    let mut expected = vec!["/b.lpc".to_string(), "/c.lpc".to_string(), "/d.lpc".to_string()];
    expected.sort();

    assert_eq!(from_a, expected);

    // Walk from B: should find D
    let from_b = tree.walk_dependents("/b.lpc");
    assert_eq!(from_b, vec!["/d.lpc".to_string()]);

    // Walk from C: should find D
    let from_c = tree.walk_dependents("/c.lpc");
    assert_eq!(from_c, vec!["/d.lpc".to_string()]);
}

#[test]
fn re_register_replaces_dependency_edges() {
    let mut tree = VersionTree::new();
    tree.register("/a.lpc", "lpc", vec![]);
    tree.register("/b.lpc", "lpc", vec![]);
    tree.register(
        "/c.lpc",
        "lpc",
        vec!["/a.lpc".to_string()],
    );

    assert_eq!(tree.get_dependents("/a.lpc"), vec!["/c.lpc"]);
    assert!(tree.get_dependents("/b.lpc").is_empty());

    // Re-register /c.lpc to depend on /b.lpc instead
    tree.register(
        "/c.lpc",
        "lpc",
        vec!["/b.lpc".to_string()],
    );

    assert!(
        tree.get_dependents("/a.lpc").is_empty(),
        "old dependency edge should be removed"
    );
    assert_eq!(tree.get_dependents("/b.lpc"), vec!["/c.lpc"]);
}

#[test]
fn re_register_preserves_version() {
    let mut tree = VersionTree::new();
    tree.register("/a.lpc", "lpc", vec![]);
    tree.bump_version("/a.lpc");
    tree.bump_version("/a.lpc");
    assert_eq!(tree.version_of("/a.lpc"), Some(3));

    // Re-register should keep version 3
    tree.register("/a.lpc", "lpc", vec![]);
    assert_eq!(tree.version_of("/a.lpc"), Some(3));
}

#[test]
fn all_programs_lists_registered() {
    let mut tree = VersionTree::new();
    tree.register("/a.lpc", "lpc", vec![]);
    tree.register("/b.rb", "ruby", vec![]);

    let mut all = tree.all_programs();
    all.sort();

    assert_eq!(all, vec!["/a.lpc", "/b.rb"]);
}

#[test]
fn language_for_returns_correct_language() {
    let mut tree = VersionTree::new();
    tree.register("/a.lpc", "lpc", vec![]);

    assert_eq!(tree.language_for("/a.lpc"), Some("lpc"));
    assert_eq!(tree.language_for("/nonexistent.lpc"), None);
}
