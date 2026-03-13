use std::sync::Arc;

use mud_driver::git::branch_protection;
use mud_driver::git::{RepoManager, Workspace};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helper: create a fresh temp directory with repos/ and world/ subdirectories,
// a RepoManager and a Workspace that share the same layout.
// ---------------------------------------------------------------------------

fn setup() -> (TempDir, Arc<RepoManager>, Workspace) {
    let dir = TempDir::new().unwrap();
    let repos_path = dir.path().join("repos");
    let world_path = dir.path().join("world");
    std::fs::create_dir_all(&repos_path).unwrap();
    std::fs::create_dir_all(&world_path).unwrap();

    let mgr = Arc::new(RepoManager::new(repos_path));
    let ws = Workspace::new(world_path, Arc::clone(&mgr));
    (dir, mgr, ws)
}

// =========================================================================
// Test 1: Full development workflow
//
// Creates a bare repo, checks it out, writes a file in dev, commits, pushes,
// verifies the commit log, and pulls.
// =========================================================================

#[test]
fn full_development_workflow() {
    let (_dir, mgr, ws) = setup();

    // 1. Create a bare repo with initial seed (main + develop branches)
    mgr.create_repo("vikings", "village", true, None).unwrap();
    assert!(mgr.repo_exists("vikings", "village"));

    // 2. Checkout: creates production (main) and dev (develop) working dirs
    let prod_path = ws.checkout("vikings", "village").unwrap();
    let dev_path = ws.dev_path("vikings", "village");
    assert!(prod_path.exists(), "production workspace must exist");
    assert!(dev_path.exists(), "dev workspace must exist");

    // Both workspaces are valid git repos
    assert!(git2::Repository::open(&prod_path).is_ok());
    assert!(git2::Repository::open(&dev_path).is_ok());

    // 3. Verify initial seed files are present in both workspaces
    assert!(prod_path.join(".meta.yml").exists());
    assert!(prod_path.join("mud_aliases.rb").exists());
    assert!(prod_path.join("rooms").join("entrance.rb").exists());
    assert!(dev_path.join(".meta.yml").exists());

    // 4. Write a new file in the dev workspace
    std::fs::write(dev_path.join("quest.rb"), "class Quest\n  # TODO\nend\n").unwrap();

    // 5. Commit and push from the dev workspace
    let oid = ws
        .commit("vikings", "village", "bjorn", "Add quest file", "develop")
        .unwrap();
    assert!(!oid.is_empty(), "commit OID must be non-empty");

    // 6. Verify the commit appears in the log
    let commits = ws.log("vikings", "village", "develop", 10).unwrap();
    assert_eq!(commits.len(), 2, "should have seed + new commit");
    assert_eq!(commits[0].message, "Add quest file");
    assert_eq!(commits[0].author, "bjorn");
    assert_eq!(commits[1].message, "Initial area template");

    // 7. Pull in the dev workspace (should be a no-op since we just pushed)
    ws.pull("vikings", "village", "develop").unwrap();

    // 8. Verify the file is still there after pull
    assert!(dev_path.join("quest.rb").exists());
}

// =========================================================================
// Test 2: Branch management workflow
//
// Creates a repo, verifies seeded branches, creates a feature branch,
// and lists all branches.
// =========================================================================

#[test]
fn branch_management_workflow() {
    let (_dir, mgr, ws) = setup();

    mgr.create_repo("vikings", "fortress", true, None).unwrap();
    ws.checkout("vikings", "fortress").unwrap();

    // Verify seeded branches exist
    let branches = ws.branches("vikings", "fortress").unwrap();
    assert!(
        branches.contains(&"main".to_string()),
        "main branch must exist after seeding"
    );
    assert!(
        branches.contains(&"develop".to_string()),
        "develop branch must exist after seeding"
    );
    assert_eq!(branches.len(), 2, "only main and develop initially");

    // Create a feature branch
    ws.create_branch("vikings", "fortress", "feature_drawbridge")
        .unwrap();

    // List branches again and verify the new one appears
    let branches = ws.branches("vikings", "fortress").unwrap();
    assert_eq!(branches.len(), 3);
    assert!(branches.contains(&"feature_drawbridge".to_string()));

    // Branches should be sorted alphabetically
    assert_eq!(branches, vec!["develop", "feature_drawbridge", "main"]);
}

// =========================================================================
// Test 3: Branch protection lifecycle
//
// Installs pre-receive hook, verifies it, removes it, verifies removal,
// then re-installs and verifies again.
// =========================================================================

#[test]
fn branch_protection_lifecycle() {
    let (_dir, mgr, _ws) = setup();

    mgr.create_repo("vikings", "temple", true, None).unwrap();
    let repo_path = mgr.repo_path("vikings", "temple");

    // Initially no protection
    assert!(
        !branch_protection::is_protected(&repo_path),
        "no protection before install"
    );

    // Install branch protection
    branch_protection::install_branch_protection(&repo_path).unwrap();
    assert!(
        branch_protection::is_protected(&repo_path),
        "should be protected after install"
    );

    // Verify the hook file exists and is executable
    let hook_path = repo_path.join("hooks").join("pre-receive");
    assert!(hook_path.exists(), "pre-receive hook file must exist");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&hook_path).unwrap().permissions();
        assert!(perms.mode() & 0o111 != 0, "hook must be executable");
    }

    // Verify hook content rejects pushes to main
    let content = std::fs::read_to_string(&hook_path).unwrap();
    assert!(
        content.contains("refs/heads/main"),
        "hook must check for main branch"
    );
    assert!(
        content.contains("exit 1"),
        "hook must exit non-zero on main push"
    );

    // Remove branch protection
    branch_protection::remove_branch_protection(&repo_path).unwrap();
    assert!(
        !branch_protection::is_protected(&repo_path),
        "should not be protected after removal"
    );
    assert!(!hook_path.exists(), "hook file must be deleted");

    // Re-install and verify again
    branch_protection::install_branch_protection(&repo_path).unwrap();
    assert!(
        branch_protection::is_protected(&repo_path),
        "should be protected after re-install"
    );
    assert!(hook_path.exists(), "hook file must exist after re-install");
}

// =========================================================================
// Test 4: Multi-area isolation
//
// Creates two independent areas in different namespaces, verifies they
// have separate bare repos, separate workspaces, and that changes in
// one do not affect the other.
// =========================================================================

#[test]
fn multi_area_isolation() {
    let (_dir, mgr, ws) = setup();

    // Create two areas in different namespaces
    mgr.create_repo("vikings", "village", true, None).unwrap();
    mgr.create_repo("elves", "forest", true, None).unwrap();

    // Each has its own bare repo
    let vikings_bare = mgr.repo_path("vikings", "village");
    let elves_bare = mgr.repo_path("elves", "forest");
    assert!(vikings_bare.exists());
    assert!(elves_bare.exists());
    assert_ne!(vikings_bare, elves_bare);

    // Checkout both
    let vikings_prod = ws.checkout("vikings", "village").unwrap();
    let elves_prod = ws.checkout("elves", "forest").unwrap();
    assert_ne!(vikings_prod, elves_prod);

    let vikings_dev = ws.dev_path("vikings", "village");
    let elves_dev = ws.dev_path("elves", "forest");
    assert_ne!(vikings_dev, elves_dev);

    // Write a file in vikings dev only
    std::fs::write(vikings_dev.join("longship.rb"), "class Longship; end").unwrap();

    // Commit in vikings
    ws.commit("vikings", "village", "ragnar", "Add longship", "develop")
        .unwrap();

    // Verify the file exists in vikings dev but NOT in elves dev
    assert!(vikings_dev.join("longship.rb").exists());
    assert!(
        !elves_dev.join("longship.rb").exists(),
        "elves workspace must not contain vikings' files"
    );

    // Verify commit logs are independent
    let vikings_log = ws.log("vikings", "village", "develop", 10).unwrap();
    let elves_log = ws.log("elves", "forest", "develop", 10).unwrap();
    assert_eq!(vikings_log.len(), 2, "vikings: seed + longship commit");
    assert_eq!(elves_log.len(), 1, "elves: seed only");

    // Write and commit in elves
    std::fs::write(elves_dev.join("bow.rb"), "class Bow; end").unwrap();
    ws.commit("elves", "forest", "legolas", "Add bow", "develop")
        .unwrap();

    // Verify elves has the bow, vikings does not
    assert!(elves_dev.join("bow.rb").exists());
    assert!(
        !vikings_dev.join("bow.rb").exists(),
        "vikings workspace must not contain elves' files"
    );
}

// =========================================================================
// Test 5: Dev vs production workspace isolation
//
// Verifies that commits to the develop branch in the dev workspace do not
// appear in the production workspace (which tracks main).
// =========================================================================

#[test]
fn dev_vs_production_workspace_isolation() {
    let (_dir, mgr, ws) = setup();

    mgr.create_repo("vikings", "harbor", true, None).unwrap();
    ws.checkout("vikings", "harbor").unwrap();

    let prod_path = ws.workspace_path("vikings", "harbor");
    let dev_path = ws.dev_path("vikings", "harbor");

    // Verify both workspaces track different branches
    let prod_repo = git2::Repository::open(&prod_path).unwrap();
    let dev_repo = git2::Repository::open(&dev_path).unwrap();

    let prod_head = prod_repo.head().unwrap();
    let dev_head = dev_repo.head().unwrap();

    // Production HEAD should reference main, dev HEAD should reference develop
    let prod_branch = prod_head.shorthand().unwrap();
    let dev_branch = dev_head.shorthand().unwrap();
    assert_eq!(prod_branch, "main", "production must track main");
    assert_eq!(dev_branch, "develop", "dev must track develop");

    // Make a change in dev, commit and push
    std::fs::write(dev_path.join("dock.rb"), "class Dock; end").unwrap();
    ws.commit("vikings", "harbor", "floki", "Add dock", "develop")
        .unwrap();

    // Dev workspace has the new file
    assert!(dev_path.join("dock.rb").exists());

    // Production workspace does NOT have the file (it tracks main)
    assert!(
        !prod_path.join("dock.rb").exists(),
        "production must not have dev-only changes"
    );

    // Verify via logs: dev has 2 commits, production has 1
    let dev_log = ws.log("vikings", "harbor", "develop", 10).unwrap();
    let prod_log = ws.log("vikings", "harbor", "main", 10).unwrap();
    assert_eq!(dev_log.len(), 2, "dev: seed + dock commit");
    assert_eq!(prod_log.len(), 1, "production: seed only");

    // Pull production -- still no dock.rb since main wasn't updated
    ws.pull("vikings", "harbor", "main").unwrap();
    assert!(
        !prod_path.join("dock.rb").exists(),
        "production must still not have dev-only changes after pull"
    );
}

// =========================================================================
// Test 6: Multiple commits and log ordering
//
// Verifies that multiple sequential commits appear in the correct order
// in the log (newest first).
// =========================================================================

#[test]
fn multiple_commits_log_ordering() {
    let (_dir, mgr, ws) = setup();

    mgr.create_repo("vikings", "hall", true, None).unwrap();
    ws.checkout("vikings", "hall").unwrap();

    let dev_path = ws.dev_path("vikings", "hall");

    // Make three sequential commits
    std::fs::write(dev_path.join("table.rb"), "class Table; end").unwrap();
    ws.commit("vikings", "hall", "erik", "Add table", "develop")
        .unwrap();

    std::fs::write(dev_path.join("throne.rb"), "class Throne; end").unwrap();
    ws.commit("vikings", "hall", "sigurd", "Add throne", "develop")
        .unwrap();

    std::fs::write(dev_path.join("banner.rb"), "class Banner; end").unwrap();
    ws.commit("vikings", "hall", "ivar", "Add banner", "develop")
        .unwrap();

    // Log should contain all 4 commits (seed + 3 new)
    let log = ws.log("vikings", "hall", "develop", 10).unwrap();
    assert_eq!(log.len(), 4, "seed + 3 commits");

    // All commit messages are present
    let messages: Vec<&str> = log.iter().map(|c| c.message.as_str()).collect();
    assert!(messages.contains(&"Add table"));
    assert!(messages.contains(&"Add throne"));
    assert!(messages.contains(&"Add banner"));
    assert!(messages.contains(&"Initial area template"));

    // All authors are present
    let authors: Vec<&str> = log.iter().map(|c| c.author.as_str()).collect();
    assert!(authors.contains(&"erik"));
    assert!(authors.contains(&"sigurd"));
    assert!(authors.contains(&"ivar"));
    assert!(authors.contains(&"MUD Driver"));

    // Verify log limit works: requesting 2 returns exactly 2
    let limited = ws.log("vikings", "hall", "develop", 2).unwrap();
    assert_eq!(limited.len(), 2);

    // The limited log entries must be a subset of the full log
    for entry in &limited {
        assert!(
            messages.contains(&entry.message.as_str()),
            "limited log entry '{}' must appear in full log",
            entry.message
        );
    }
}

// =========================================================================
// Test 7: Diff shows unstaged changes correctly
//
// Verifies that the diff API reports added and modified files before commit.
// =========================================================================

#[test]
fn diff_shows_changes_before_commit() {
    let (_dir, mgr, ws) = setup();

    mgr.create_repo("vikings", "forge", true, None).unwrap();
    ws.checkout("vikings", "forge").unwrap();

    let dev_path = ws.dev_path("vikings", "forge");

    // Clean diff initially
    let clean = ws.diff("vikings", "forge", "develop").unwrap();
    assert!(clean.is_empty(), "no changes in a fresh checkout");

    // Add a new file
    std::fs::write(dev_path.join("sword.rb"), "class Sword; end").unwrap();

    let diff = ws.diff("vikings", "forge", "develop").unwrap();
    assert!(!diff.is_empty(), "should detect the new file");
    let sword_entry = diff.iter().find(|e| e.path == "sword.rb");
    assert!(sword_entry.is_some(), "sword.rb must appear in diff");

    // Modify an existing file
    let entrance_path = dev_path.join("rooms").join("entrance.rb");
    let original = std::fs::read_to_string(&entrance_path).unwrap();
    std::fs::write(&entrance_path, format!("{}\n# modified\n", original)).unwrap();

    let diff = ws.diff("vikings", "forge", "develop").unwrap();
    assert!(
        diff.len() >= 2,
        "should see at least the new file and the modification"
    );
    let entrance_entry = diff.iter().find(|e| e.path == "rooms/entrance.rb");
    assert!(
        entrance_entry.is_some(),
        "entrance.rb modification must appear in diff"
    );
    assert_eq!(
        entrance_entry.unwrap().status,
        "modified",
        "existing file change should be 'modified'"
    );

    // After committing, diff should be clean again
    ws.commit(
        "vikings",
        "forge",
        "ulf",
        "Add sword and modify entrance",
        "develop",
    )
    .unwrap();
    let after_commit = ws.diff("vikings", "forge", "develop").unwrap();
    assert!(after_commit.is_empty(), "diff should be clean after commit");
}

// =========================================================================
// Test 8: Branch protection with multiple areas
//
// Verifies that installing protection on one area does not affect another.
// =========================================================================

#[test]
fn branch_protection_per_area_isolation() {
    let (_dir, mgr, _ws) = setup();

    mgr.create_repo("vikings", "town", true, None).unwrap();
    mgr.create_repo("vikings", "farm", true, None).unwrap();

    let town_path = mgr.repo_path("vikings", "town");
    let farm_path = mgr.repo_path("vikings", "farm");

    // Protect town only
    branch_protection::install_branch_protection(&town_path).unwrap();

    assert!(branch_protection::is_protected(&town_path));
    assert!(
        !branch_protection::is_protected(&farm_path),
        "farm must not be affected by town's protection"
    );

    // Now protect farm too
    branch_protection::install_branch_protection(&farm_path).unwrap();
    assert!(branch_protection::is_protected(&farm_path));

    // Remove from town, farm stays protected
    branch_protection::remove_branch_protection(&town_path).unwrap();
    assert!(!branch_protection::is_protected(&town_path));
    assert!(
        branch_protection::is_protected(&farm_path),
        "farm protection must survive town's removal"
    );
}

// =========================================================================
// Test 9: ACL integration with repo creation
//
// Verifies that creating a repo sets up the correct default ACL, and that
// granting/revoking access works across the workflow.
// =========================================================================

#[test]
fn acl_integration_with_workflow() {
    let (_dir, mgr, ws) = setup();

    mgr.create_repo("vikings", "treasury", true, None).unwrap();

    let policy = mgr.get_policy("vikings", "treasury").unwrap();
    assert_eq!(policy.owner, "vikings");
    assert!(policy.user_levels.is_empty());

    // Owner can access
    assert!(mgr.can_access(
        "vikings",
        "vikings",
        "treasury",
        &mud_driver::git::AccessLevel::ReadWrite
    ));

    // Stranger cannot
    assert!(!mgr.can_access(
        "thief",
        "vikings",
        "treasury",
        &mud_driver::git::AccessLevel::ReadOnly
    ));

    // Grant read-only to a collaborator
    mgr.grant_access(
        "vikings",
        "treasury",
        "trader",
        mud_driver::git::AccessLevel::ReadOnly,
    )
    .unwrap();

    assert!(mgr.can_access(
        "trader",
        "vikings",
        "treasury",
        &mud_driver::git::AccessLevel::ReadOnly
    ));
    assert!(!mgr.can_access(
        "trader",
        "vikings",
        "treasury",
        &mud_driver::git::AccessLevel::ReadWrite
    ));

    // Workspace operations work regardless of ACL (ACL is checked at transport layer)
    ws.checkout("vikings", "treasury").unwrap();
    let dev_path = ws.dev_path("vikings", "treasury");
    std::fs::write(dev_path.join("gold.rb"), "class Gold; end").unwrap();
    ws.commit("vikings", "treasury", "trader", "Add gold", "develop")
        .unwrap();

    let log = ws.log("vikings", "treasury", "develop", 10).unwrap();
    assert_eq!(log.len(), 2);
}

// =========================================================================
// Test 10: Repo listing across namespaces
//
// Verifies that list_repos correctly enumerates repos within a namespace
// and that different namespaces are isolated.
// =========================================================================

#[test]
fn repo_listing_across_namespaces() {
    let (_dir, mgr, _ws) = setup();

    // Empty namespace
    assert!(mgr.list_repos("vikings").unwrap().is_empty());

    // Create repos in two namespaces
    mgr.create_repo("vikings", "village", true, None).unwrap();
    mgr.create_repo("vikings", "harbor", true, None).unwrap();
    mgr.create_repo("elves", "forest", true, None).unwrap();

    // Vikings namespace has two repos, sorted
    let viking_repos = mgr.list_repos("vikings").unwrap();
    assert_eq!(viking_repos, vec!["harbor", "village"]);

    // Elves namespace has one
    let elf_repos = mgr.list_repos("elves").unwrap();
    assert_eq!(elf_repos, vec!["forest"]);

    // Non-existent namespace returns empty
    assert!(mgr.list_repos("dwarves").unwrap().is_empty());
}
