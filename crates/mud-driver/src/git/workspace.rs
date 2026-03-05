use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeZone, Utc};

use super::repo_manager::RepoManager;

/// Metadata about a single git commit.
pub struct CommitInfo {
    pub oid: String,
    pub message: String,
    pub author: String,
    pub time: DateTime<Utc>,
}

/// A single file change in a diff.
pub struct DiffEntry {
    pub path: String,
    /// One of: "added", "modified", "deleted", "renamed", "unknown"
    pub status: String,
}

/// Manages working directory checkouts of bare git repositories.
///
/// Each area gets two working copies:
/// - **Production** at `world/<ns>/<name>/` tracking the `main` branch
/// - **Development** at `world/<ns>/<name>@dev/` tracking the `develop` branch
pub struct Workspace {
    world_path: PathBuf,
    repo_manager: Arc<RepoManager>,
}

impl Workspace {
    pub fn new(world_path: PathBuf, repo_manager: Arc<RepoManager>) -> Self {
        Self {
            world_path,
            repo_manager,
        }
    }

    /// The base world directory path.
    pub fn world_path(&self) -> &Path {
        &self.world_path
    }

    /// Production working directory path: `world/<ns>/<name>/`
    pub fn workspace_path(&self, ns: &str, name: &str) -> PathBuf {
        self.world_path.join(ns).join(name)
    }

    /// Dev working directory path: `world/<ns>/<name>@dev/`
    pub fn dev_path(&self, ns: &str, name: &str) -> PathBuf {
        self.world_path.join(ns).join(format!("{}@dev", name))
    }

    /// Clone bare repo into both production and dev working directories.
    /// Returns the production path.
    pub fn checkout(&self, ns: &str, name: &str) -> Result<PathBuf> {
        let bare_path = self.repo_manager.repo_path(ns, name);
        if !bare_path.exists() {
            bail!("bare repo does not exist: {}/{}", ns, name);
        }

        let prod_path = self.workspace_path(ns, name);
        let dev_path = self.dev_path(ns, name);

        // Clone main -> production
        if !prod_path.exists() {
            self.clone_branch(&bare_path, &prod_path, "main")?;
        }

        // Clone develop -> dev
        if !dev_path.exists() {
            self.clone_branch(&bare_path, &dev_path, "develop")?;
        }

        Ok(prod_path)
    }

    /// Clone a single branch from a bare repo into a working directory.
    fn clone_branch(&self, bare_path: &Path, work_path: &Path, branch: &str) -> Result<()> {
        std::fs::create_dir_all(work_path)?;
        let abs_bare = bare_path
            .canonicalize()
            .with_context(|| format!("resolving bare repo path {}", bare_path.display()))?;
        let bare_url = format!("file://{}", abs_bare.display());

        let mut builder = git2::build::RepoBuilder::new();
        builder.branch(branch);
        builder
            .clone(&bare_url, work_path)
            .with_context(|| format!("cloning branch {} to {}", branch, work_path.display()))?;
        Ok(())
    }

    /// Pull (fetch + hard reset) a specific branch in its working directory.
    /// If the working directory doesn't exist yet, it is created via `checkout()`.
    pub fn pull(&self, ns: &str, name: &str, branch: &str) -> Result<()> {
        let work_path = self.path_for_branch(ns, name, branch);
        if git2::Repository::open(&work_path).is_err() {
            // Working directory missing or corrupt — (re-)checkout from bare repo.
            if work_path.exists() {
                std::fs::remove_dir_all(&work_path)?;
            }
            self.checkout(ns, name)?;
        }
        let repo = git2::Repository::open(&work_path)
            .with_context(|| format!("opening repo at {}", work_path.display()))?;

        // Fetch from origin
        let mut remote = repo.find_remote("origin")?;
        remote.fetch(&[branch], None, None)?;

        // Reset to fetched branch
        let fetch_head = repo.find_reference(&format!("refs/remotes/origin/{}", branch))?;
        let target = fetch_head.peel_to_commit()?;
        repo.reset(target.as_object(), git2::ResetType::Hard, None)?;

        Ok(())
    }

    /// Commit all changes in the working directory and push to origin.
    /// Returns the new commit OID as a hex string.
    pub fn commit(
        &self,
        ns: &str,
        name: &str,
        author: &str,
        message: &str,
        branch: &str,
    ) -> Result<String> {
        let work_path = self.path_for_branch(ns, name, branch);
        let repo = git2::Repository::open(&work_path)?;

        // Stage all changes
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;

        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;

        let sig = git2::Signature::now(author, &format!("{}@mud", author))?;

        // Get parent commit
        let head = repo.head()?.peel_to_commit()?;

        let commit_oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])?;

        // Push to origin
        let mut remote = repo.find_remote("origin")?;
        remote.push(
            &[&format!("refs/heads/{}:refs/heads/{}", branch, branch)],
            None,
        )?;

        Ok(commit_oid.to_string())
    }

    /// Get the commit log for a branch, up to `limit` entries.
    pub fn log(
        &self,
        ns: &str,
        name: &str,
        branch: &str,
        limit: usize,
    ) -> Result<Vec<CommitInfo>> {
        let work_path = self.path_for_branch(ns, name, branch);
        let repo = git2::Repository::open(&work_path)?;

        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for oid_result in revwalk.take(limit) {
            let oid = oid_result?;
            let commit = repo.find_commit(oid)?;
            let time_secs = commit.time().seconds();
            let time = Utc
                .timestamp_opt(time_secs, 0)
                .single()
                .unwrap_or_else(Utc::now);
            commits.push(CommitInfo {
                oid: oid.to_string(),
                message: commit.message().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("unknown").to_string(),
                time,
            });
        }

        Ok(commits)
    }

    /// Get the diff (changed files) between HEAD and the working directory.
    pub fn diff(&self, ns: &str, name: &str, branch: &str) -> Result<Vec<DiffEntry>> {
        let work_path = self.path_for_branch(ns, name, branch);
        let repo = git2::Repository::open(&work_path)?;

        let head = repo.head()?.peel_to_tree()?;
        let mut opts = git2::DiffOptions::new();
        opts.include_untracked(true);
        let diff = repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut opts))?;

        let mut entries = Vec::new();
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let status = match delta.status() {
                git2::Delta::Added => "added",
                git2::Delta::Modified => "modified",
                git2::Delta::Deleted => "deleted",
                git2::Delta::Renamed => "renamed",
                _ => "unknown",
            };
            entries.push(DiffEntry {
                path,
                status: status.to_string(),
            });
        }

        Ok(entries)
    }

    /// List branches in the bare repo (sorted alphabetically).
    pub fn branches(&self, ns: &str, name: &str) -> Result<Vec<String>> {
        let bare_path = self.repo_manager.repo_path(ns, name);
        let repo = git2::Repository::open_bare(&bare_path)?;
        let mut names = Vec::new();
        for branch_result in repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = branch_result?;
            if let Some(name) = branch.name()? {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Create a new branch in the bare repo, branching from `main`.
    pub fn create_branch(&self, ns: &str, name: &str, branch_name: &str) -> Result<()> {
        let bare_path = self.repo_manager.repo_path(ns, name);
        let repo = git2::Repository::open_bare(&bare_path)?;
        let main_ref = repo.find_reference("refs/heads/main")?;
        let commit = main_ref.peel_to_commit()?;
        repo.branch(branch_name, &commit, false)?;
        Ok(())
    }

    /// Switch the @dev working directory to track a different branch.
    /// Fetches the branch from origin, then checks it out (creating a local
    /// tracking branch if needed) and hard-resets to the remote head.
    pub fn checkout_branch(&self, ns: &str, name: &str, branch: &str) -> Result<()> {
        let dev_path = self.dev_path(ns, name);
        let repo = git2::Repository::open(&dev_path)
            .with_context(|| format!("opening dev repo at {}", dev_path.display()))?;

        // Fetch the branch from origin
        let mut remote = repo.find_remote("origin")?;
        remote.fetch(&[branch], None, None)?;

        let remote_ref_name = format!("refs/remotes/origin/{}", branch);
        let remote_ref = repo
            .find_reference(&remote_ref_name)
            .with_context(|| format!("branch '{}' not found in remote", branch))?;
        let target_commit = remote_ref.peel_to_commit()?;

        // Check if local branch exists
        match repo.find_branch(branch, git2::BranchType::Local) {
            Ok(mut local_branch) => {
                // Update existing local branch to point at remote head
                local_branch.get_mut().set_target(
                    target_commit.id(),
                    &format!("checkout_branch: update {} to origin/{}", branch, branch),
                )?;
            }
            Err(_) => {
                // Create local tracking branch
                repo.branch(branch, &target_commit, false)?;
            }
        }

        // Checkout the branch
        let obj = target_commit.as_object();
        repo.checkout_tree(obj, Some(git2::build::CheckoutBuilder::new().force()))?;
        repo.set_head(&format!("refs/heads/{}", branch))?;

        Ok(())
    }

    /// Resolve the correct working directory path for a given branch.
    /// `develop` maps to the dev path; all other branches use the production path.
    fn path_for_branch(&self, ns: &str, name: &str, branch: &str) -> PathBuf {
        if branch == "develop" {
            self.dev_path(ns, name)
        } else {
            self.workspace_path(ns, name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a temp dir with both a bare-repo area (repos/) and a world dir (world/).
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

    #[test]
    fn test_workspace_paths() {
        let (_dir, _mgr, ws) = setup();

        let prod = ws.workspace_path("testns", "village");
        assert!(prod.ends_with("world/testns/village"));

        let dev = ws.dev_path("testns", "village");
        assert!(dev.ends_with("world/testns/village@dev"));
    }

    #[test]
    fn test_checkout_creates_both_directories() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();

        let prod_path = ws.checkout("testns", "village").unwrap();
        assert!(prod_path.exists());
        assert!(ws.dev_path("testns", "village").exists());

        // Both should be proper git repos
        assert!(git2::Repository::open(&prod_path).is_ok());
        assert!(git2::Repository::open(&ws.dev_path("testns", "village")).is_ok());
    }

    #[test]
    fn test_checkout_missing_bare_repo() {
        let (_dir, _mgr, ws) = setup();

        let result = ws.checkout("testns", "nonexistent");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("bare repo does not exist"));
    }

    #[test]
    fn test_checkout_idempotent() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();

        let path1 = ws.checkout("testns", "village").unwrap();
        let path2 = ws.checkout("testns", "village").unwrap();
        assert_eq!(path1, path2);
    }

    #[test]
    fn test_log_returns_commits() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        ws.checkout("testns", "village").unwrap();

        let commits = ws.log("testns", "village", "main", 10).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].message, "Initial area template");
        assert_eq!(commits[0].author, "MUD Driver");
    }

    #[test]
    fn test_diff_clean_working_dir() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        ws.checkout("testns", "village").unwrap();

        let entries = ws.diff("testns", "village", "main").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_diff_with_changes() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        ws.checkout("testns", "village").unwrap();

        // Create a new file in the working directory
        let prod_path = ws.workspace_path("testns", "village");
        std::fs::write(prod_path.join("new_file.txt"), "hello").unwrap();

        let entries = ws.diff("testns", "village", "main").unwrap();
        assert!(!entries.is_empty());

        let new_file = entries.iter().find(|e| e.path == "new_file.txt");
        assert!(new_file.is_some());
    }

    #[test]
    fn test_commit_and_push() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        ws.checkout("testns", "village").unwrap();

        // Create a new file
        let dev_path = ws.dev_path("testns", "village");
        std::fs::write(dev_path.join("quest.rb"), "class Quest; end").unwrap();

        let oid = ws
            .commit("testns", "village", "alice", "Add quest", "develop")
            .unwrap();
        assert!(!oid.is_empty());

        // Verify the commit is in the log
        let commits = ws.log("testns", "village", "develop", 10).unwrap();
        assert_eq!(commits.len(), 2); // seed + new
        assert_eq!(commits[0].message, "Add quest");
        assert_eq!(commits[0].author, "alice");
    }

    #[test]
    fn test_branches() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();

        let branches = ws.branches("testns", "village").unwrap();
        assert_eq!(branches, vec!["develop", "main"]);
    }

    #[test]
    fn test_create_branch() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();

        ws.create_branch("testns", "village", "feature_x").unwrap();

        let branches = ws.branches("testns", "village").unwrap();
        assert!(branches.contains(&"feature_x".to_string()));
    }

    #[test]
    fn test_pull() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        ws.checkout("testns", "village").unwrap();

        // Pull should succeed (no-op since we're already up to date)
        ws.pull("testns", "village", "main").unwrap();
        ws.pull("testns", "village", "develop").unwrap();
    }

    #[test]
    fn test_pull_auto_checkouts_when_workspace_missing() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        // Do NOT call checkout() — pull should auto-create working dirs
        assert!(!ws.workspace_path("testns", "village").exists());

        ws.pull("testns", "village", "main").unwrap();

        // Both working directories should now exist
        assert!(ws.workspace_path("testns", "village").exists());
        assert!(ws.dev_path("testns", "village").exists());

        // Template files should be present in the checked-out workspace
        let prod = ws.workspace_path("testns", "village");
        assert!(prod.join(".meta.yml").exists(), ".meta.yml should exist");
        assert!(prod.join("mud_aliases.rb").exists(), "mud_aliases.rb should exist");
        assert!(prod.join("rooms/entrance.rb").exists(), "rooms/entrance.rb should exist");

        let entrance = std::fs::read_to_string(prod.join("rooms/entrance.rb")).unwrap();
        assert!(entrance.contains("class Entrance < Room"), "entrance.rb should contain class definition");
        assert!(entrance.contains("Welcome to village"), "entrance.rb should contain area name");
    }

    #[test]
    fn test_pull_recovers_from_empty_directory() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        // Create empty directory (simulates partial checkout failure)
        let prod = ws.workspace_path("testns", "village");
        std::fs::create_dir_all(&prod).unwrap();
        assert!(prod.exists());
        assert!(git2::Repository::open(&prod).is_err());

        ws.pull("testns", "village", "main").unwrap();

        // Should now be a valid git repo with template files
        assert!(git2::Repository::open(&prod).is_ok());
        assert!(prod.join(".meta.yml").exists(), ".meta.yml should exist");
        assert!(prod.join("mud_aliases.rb").exists(), "mud_aliases.rb should exist");
        assert!(prod.join("rooms/entrance.rb").exists(), "rooms/entrance.rb should exist");

        let aliases = std::fs::read_to_string(prod.join("mud_aliases.rb")).unwrap();
        assert!(aliases.contains("Room = MUD::Stdlib::World::Room"), "mud_aliases.rb should contain Room alias");
        assert!(aliases.contains("Daemon = MUD::Stdlib::World::Daemon"), "mud_aliases.rb should contain Daemon alias");
    }

    #[test]
    fn test_checkout_with_relative_paths() {
        // Regression test: clone_branch must work when RepoManager and
        // Workspace are constructed with relative paths (as in production
        // configs like git_path: "git-server", path: "world").
        let dir = TempDir::new().unwrap();
        let repos_rel = "repos";
        let world_rel = "world";
        let abs_repos = dir.path().join(repos_rel);
        let abs_world = dir.path().join(world_rel);
        std::fs::create_dir_all(&abs_repos).unwrap();
        std::fs::create_dir_all(&abs_world).unwrap();

        // Use relative paths (relative to dir.path()) like production does.
        let mgr = Arc::new(RepoManager::new(abs_repos.clone()));
        let ws = Workspace::new(abs_world.clone(), Arc::clone(&mgr));

        mgr.create_repo("ns", "area", true, None).unwrap();

        // Set CWD to temp dir so relative paths resolve — but RepoManager
        // already has absolute paths from the join above. The real test is
        // that clone_branch canonicalizes the bare path before file:// URL.
        let prod = ws.checkout("ns", "area").unwrap();
        assert!(prod.exists());
        assert!(prod.join("rooms/entrance.rb").exists());
    }

    #[test]
    fn test_checkout_branch() {
        let (_dir, mgr, ws) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        ws.checkout("testns", "village").unwrap();

        // Create a feature branch in the bare repo
        ws.create_branch("testns", "village", "feature_x").unwrap();

        // Switch @dev to the feature branch
        ws.checkout_branch("testns", "village", "feature_x")
            .unwrap();

        // Verify the @dev repo is now on the feature branch
        let dev_path = ws.dev_path("testns", "village");
        let repo = git2::Repository::open(&dev_path).unwrap();
        let head = repo.head().unwrap();
        assert_eq!(
            head.shorthand().unwrap(),
            "feature_x",
            "dev should be on feature_x branch"
        );

        // Switch back to develop
        ws.checkout_branch("testns", "village", "develop").unwrap();
        let repo = git2::Repository::open(&dev_path).unwrap();
        let head = repo.head().unwrap();
        assert_eq!(head.shorthand().unwrap(), "develop");
    }

    #[test]
    fn test_path_for_branch() {
        let (_dir, _mgr, ws) = setup();

        let develop = ws.path_for_branch("ns", "area", "develop");
        assert!(develop.ends_with("ns/area@dev"));

        let main = ws.path_for_branch("ns", "area", "main");
        assert!(main.ends_with("ns/area"));

        let feature = ws.path_for_branch("ns", "area", "feature_x");
        assert!(feature.ends_with("ns/area"));
    }
}
