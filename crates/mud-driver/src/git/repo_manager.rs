use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Access level for a collaborator on a repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AccessLevel {
    #[serde(rename = "read_only")]
    ReadOnly,
    #[serde(rename = "read_write")]
    ReadWrite,
}

/// YAML-serialized access policy for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoPolicy {
    pub owner: String,
    #[serde(default)]
    pub users: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub user_levels: HashMap<String, AccessLevel>,
    #[serde(default)]
    pub role_levels: HashMap<String, AccessLevel>,
}

/// Manages bare git repositories and their YAML-based access policies.
///
/// Each repository lives at `base_path/<ns>/<name>.git` (bare) with an
/// accompanying `<name>.git.policy.yml` file for access control.
pub struct RepoManager {
    base_path: PathBuf,
}

impl RepoManager {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Path to bare repo: `base_path/ns/name.git`
    pub fn repo_path(&self, ns: &str, name: &str) -> PathBuf {
        self.base_path.join(ns).join(format!("{}.git", name))
    }

    /// Path to policy file: `base_path/ns/name.git.policy.yml`
    fn policy_path(&self, ns: &str, name: &str) -> PathBuf {
        self.base_path
            .join(ns)
            .join(format!("{}.git.policy.yml", name))
    }

    /// Create a bare git repo with initial commit and both `main` + `develop` branches.
    ///
    /// If `seed` is true, an initial commit is created with template files on
    /// the `main` branch, and a `develop` branch pointing to the same commit.
    ///
    /// When `template_files` is `Some`, those files are used as the initial
    /// content. Template variables `{{namespace}}` and `{{area_name}}` are
    /// substituted in each file. When `None`, a minimal default template
    /// (`.meta.yml`, `mud_aliases.rb`, `rooms/entrance.rb`) is used.
    pub fn create_repo(
        &self,
        ns: &str,
        name: &str,
        seed: bool,
        template_files: Option<&HashMap<String, String>>,
    ) -> Result<()> {
        Self::validate_name(ns)?;
        Self::validate_name(name)?;

        let repo_path = self.repo_path(ns, name);
        if repo_path.exists() {
            bail!("repository already exists: {}/{}", ns, name);
        }

        // Create parent directories
        std::fs::create_dir_all(repo_path.parent().unwrap())?;

        // Initialize bare repository
        let repo =
            git2::Repository::init_bare(&repo_path).context("initializing bare repository")?;

        if seed {
            self.seed_initial_commit(&repo, ns, name, template_files)?;
        }

        let policy = RepoPolicy {
            owner: ns.to_string(),
            users: Vec::new(),
            roles: Vec::new(),
            user_levels: HashMap::new(),
            role_levels: HashMap::new(),
        };
        self.write_policy(ns, name, &policy)?;

        Ok(())
    }

    /// Create a new repo using the contents of another repository's branch as
    /// the initial template, but with fresh history in the destination repo.
    pub fn create_repo_from_template_repo(
        &self,
        ns: &str,
        name: &str,
        template_repo_path: &Path,
        source_branch: &str,
    ) -> Result<()> {
        let files = Self::template_files_from_repo(template_repo_path, source_branch)?;
        self.create_repo(ns, name, true, Some(&files))
    }

    /// Seed initial commit with template files on `main` branch, then create `develop` branch.
    ///
    /// Template variables `{{namespace}}` and `{{area_name}}` are replaced in
    /// every file's content. If `template_files` is `None`, a minimal default
    /// set is used instead.
    fn seed_initial_commit(
        &self,
        repo: &git2::Repository,
        ns: &str,
        name: &str,
        template_files: Option<&HashMap<String, String>>,
    ) -> Result<()> {
        let sig = git2::Signature::now("MUD Driver", "mud@localhost")?;
        let mut index = repo.index()?;

        let files = Self::seed_files(ns, template_files);

        // Sort keys for deterministic tree ordering
        let mut paths: Vec<&String> = files.keys().collect();
        paths.sort();

        for path in paths {
            let raw_content = &files[path];
            let content = raw_content
                .replace("{{namespace}}", ns)
                .replace("{{area_name}}", name);
            Self::add_blob_to_index(repo, &mut index, path.as_bytes(), content.as_bytes())?;
        }

        // Write index to a tree object in the repo's object database
        let tree_oid = index.write_tree_to(repo)?;
        let tree = repo.find_tree(tree_oid)?;

        // Create initial commit on main
        let commit_oid = repo.commit(
            Some("refs/heads/main"),
            &sig,
            &sig,
            "Initial area template",
            &tree,
            &[], // no parents — this is the root commit
        )?;

        // Create develop branch pointing to the same commit
        let commit = repo.find_commit(commit_oid)?;
        repo.branch("develop", &commit, false)?;

        // Set HEAD to main
        repo.set_head("refs/heads/main")?;

        Ok(())
    }

    fn seed_files(
        ns: &str,
        template_files: Option<&HashMap<String, String>>,
    ) -> HashMap<String, String> {
        let mut files = template_files.cloned().unwrap_or_else(Self::default_template_files);
        files
            .entry(".meta.yml".into())
            .or_insert_with(|| format!("owner: {ns}\n"));
        files
    }

    /// Add a blob to the in-memory index at the given path.
    fn add_blob_to_index(
        repo: &git2::Repository,
        index: &mut git2::Index,
        path: &[u8],
        content: &[u8],
    ) -> Result<()> {
        let oid = repo.blob(content)?;
        index.add(&git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: content.len() as u32,
            id: oid,
            flags: 0,
            flags_extended: 0,
            path: path.to_vec(),
        })?;
        Ok(())
    }

    /// Minimal default template used when no adapter-provided template is available.
    fn default_template_files() -> HashMap<String, String> {
        let mut files = HashMap::new();
        files.insert(".meta.yml".into(), "owner: {{namespace}}\n".into());
        files.insert(
            "mud_aliases.rb".into(),
            concat!(
                "Room = MUD::Stdlib::World::Room\n",
                "Item = MUD::Stdlib::World::Item\n",
                "NPC = MUD::Stdlib::World::NPC\n",
                "Daemon = MUD::Stdlib::World::Daemon\n",
            )
            .into(),
        );
        files.insert(
            "rooms/entrance.rb".into(),
            concat!(
                "class Entrance < Room\n",
                "  title \"The Entrance\"\n",
                "  description \"Welcome to {{area_name}}.\"\n",
                "  exit :north, to: \"hall\"\n",
                "end\n",
            )
            .into(),
        );
        files
    }

    fn template_files_from_repo(
        template_repo_path: &Path,
        source_branch: &str,
    ) -> Result<HashMap<String, String>> {
        let repo = git2::Repository::open(template_repo_path)
            .or_else(|_| git2::Repository::open_bare(template_repo_path))
            .with_context(|| {
                format!(
                    "opening template repository at {}",
                    template_repo_path.display()
                )
            })?;

        let branch_candidates = [
            format!("refs/heads/{source_branch}"),
            format!("refs/remotes/origin/{source_branch}"),
        ];
        let mut commit = None;
        for reference_name in &branch_candidates {
            if let Ok(reference) = repo.find_reference(reference_name) {
                commit = Some(reference.peel_to_commit()?);
                break;
            }
        }
        let commit = commit.with_context(|| {
            format!(
                "finding source branch '{}' in template repo {}",
                source_branch,
                template_repo_path.display()
            )
        })?;

        let tree = commit.tree()?;
        let mut files = HashMap::new();
        Self::collect_tree_files(&repo, "", &tree, &mut files)?;

        if !files.contains_key(".meta.yml") {
            files.insert(".meta.yml".into(), "owner: {{namespace}}\n".into());
        }

        Ok(files)
    }

    fn collect_tree_files(
        repo: &git2::Repository,
        prefix: &str,
        tree: &git2::Tree<'_>,
        files: &mut HashMap<String, String>,
    ) -> Result<()> {
        for entry in tree {
            let Some(name) = entry.name() else {
                continue;
            };
            let path = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };

            match entry.kind() {
                Some(git2::ObjectType::Blob) => {
                    if path == ".gitkeep" || path.ends_with("/.gitkeep") {
                        continue;
                    }
                    let blob = repo.find_blob(entry.id())?;
                    let content = std::str::from_utf8(blob.content()).with_context(|| {
                        format!("template file '{}' is not valid UTF-8", path)
                    })?;
                    files.insert(path, content.to_string());
                }
                Some(git2::ObjectType::Tree) => {
                    let subtree = repo.find_tree(entry.id())?;
                    Self::collect_tree_files(repo, &path, &subtree, files)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Check whether a repository exists on disk.
    pub fn repo_exists(&self, ns: &str, name: &str) -> bool {
        self.repo_path(ns, name).exists()
    }

    /// List all repository names within a namespace (sorted).
    pub fn list_repos(&self, ns: &str) -> Result<Vec<String>> {
        let ns_path = self.base_path.join(ns);
        if !ns_path.exists() {
            return Ok(Vec::new());
        }

        let mut repos = Vec::new();
        for entry in std::fs::read_dir(&ns_path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".git") && entry.path().is_dir() {
                repos.push(name.trim_end_matches(".git").to_string());
            }
        }
        repos.sort();
        Ok(repos)
    }

    /// Delete a repository and its policy file.
    pub fn delete_repo(&self, ns: &str, name: &str) -> Result<()> {
        let repo_path = self.repo_path(ns, name);
        if repo_path.exists() {
            std::fs::remove_dir_all(&repo_path)?;
        }
        let policy_path = self.policy_path(ns, name);
        if policy_path.exists() {
            std::fs::remove_file(&policy_path)?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // ACL management
    // -----------------------------------------------------------------------

    /// Check whether `username` has the requested `level` of access to a repo.
    pub fn can_access(&self, username: &str, ns: &str, name: &str, level: &AccessLevel) -> bool {
        if let Ok(policy) = self.get_policy(ns, name) {
            return self.policy_allows(&policy, username, level);
        }
        false
    }

    /// Grant a collaborator access at the given level.
    pub fn grant_access(
        &self,
        ns: &str,
        name: &str,
        username: &str,
        level: AccessLevel,
    ) -> Result<()> {
        let mut policy = self.get_policy(ns, name)?;
        policy.users.retain(|user| user != username);
        policy.user_levels.insert(username.to_string(), level);
        self.write_policy(ns, name, &policy)
    }

    /// Revoke a collaborator's access entirely.
    pub fn revoke_access(&self, ns: &str, name: &str, username: &str) -> Result<()> {
        let mut policy = self.get_policy(ns, name)?;
        policy.users.retain(|user| user != username);
        policy.user_levels.remove(username);
        self.write_policy(ns, name, &policy)
    }

    /// Read the repo access policy. Returns a default (owner = ns, no grants)
    /// if the policy file does not exist.
    pub fn get_policy(&self, ns: &str, name: &str) -> Result<RepoPolicy> {
        let path = self.policy_path(ns, name);
        if !path.exists() {
            return Ok(RepoPolicy {
                owner: ns.to_string(),
                users: Vec::new(),
                roles: Vec::new(),
                user_levels: HashMap::new(),
                role_levels: HashMap::new(),
            });
        }
        let content = std::fs::read_to_string(&path)?;
        let policy: RepoPolicy = serde_yaml::from_str(&content)?;
        Ok(policy)
    }

    fn write_policy(&self, ns: &str, name: &str, policy: &RepoPolicy) -> Result<()> {
        let path = self.policy_path(ns, name);
        let content = serde_yaml::to_string(policy)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    fn policy_allows(&self, policy: &RepoPolicy, username: &str, level: &AccessLevel) -> bool {
        if policy.owner == username || policy.users.iter().any(|user| user == username) {
            return true;
        }

        match policy.user_levels.get(username) {
            Some(AccessLevel::ReadWrite) => true,
            Some(AccessLevel::ReadOnly) => *level == AccessLevel::ReadOnly,
            None => false,
        }
    }

    /// Validate a namespace or repo name: must be non-empty and match `[a-z0-9_]+`.
    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            bail!("invalid name '{}': must match [a-z0-9_]+", name);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, RepoManager) {
        let dir = TempDir::new().unwrap();
        let mgr = RepoManager::new(dir.path().to_path_buf());
        (dir, mgr)
    }

    #[test]
    fn test_validate_name() {
        assert!(RepoManager::validate_name("hello").is_ok());
        assert!(RepoManager::validate_name("area_01").is_ok());
        assert!(RepoManager::validate_name("").is_err());
        assert!(RepoManager::validate_name("Hello").is_err());
        assert!(RepoManager::validate_name("my-repo").is_err());
        assert!(RepoManager::validate_name("has space").is_err());
    }

    #[test]
    fn test_create_repo_seeded() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        assert!(mgr.repo_exists("testns", "village"));

        // Verify it's a valid bare repo with both branches
        let repo_path = mgr.repo_path("testns", "village");
        let repo = git2::Repository::open_bare(&repo_path).unwrap();
        assert!(repo.is_bare());

        // Check main branch exists
        let main_ref = repo.find_reference("refs/heads/main").unwrap();
        assert!(main_ref.target().is_some());

        // Check develop branch exists and points to the same commit
        let develop_ref = repo.find_reference("refs/heads/develop").unwrap();
        assert_eq!(main_ref.target(), develop_ref.target());

        // Check that the commit has the expected tree entries
        let commit = repo.find_commit(main_ref.target().unwrap()).unwrap();
        let tree = commit.tree().unwrap();
        assert!(tree.get_name(".meta.yml").is_some());
        assert!(tree.get_name("mud_aliases.rb").is_some());
        assert!(tree.get_name("rooms").is_some());

        // Check commit message
        assert_eq!(commit.message().unwrap(), "Initial area template");
    }

    #[test]
    fn test_create_repo_seeded_with_custom_template() {
        let (_dir, mgr) = setup();

        let mut template = HashMap::new();
        template.insert(".meta.yml".into(), "owner: {{namespace}}\n".into());
        template.insert("mud_loader.rb".into(), "loader { }\n".into());
        template.insert(
            "README.md".into(),
            "# {{area_name}}\nBy {{namespace}}\n".into(),
        );
        template.insert("items/.gitkeep".into(), String::new());

        mgr.create_repo("testns", "castle", true, Some(&template))
            .unwrap();

        let repo_path = mgr.repo_path("testns", "castle");
        let repo = git2::Repository::open_bare(&repo_path).unwrap();
        let main_ref = repo.find_reference("refs/heads/main").unwrap();
        let commit = repo.find_commit(main_ref.target().unwrap()).unwrap();
        let tree = commit.tree().unwrap();

        // Verify custom template files are present
        assert!(tree.get_name(".meta.yml").is_some());
        assert!(tree.get_name("mud_loader.rb").is_some());
        assert!(tree.get_name("README.md").is_some());
        assert!(tree.get_name("items").is_some());

        // Verify template substitution happened
        let meta_entry = tree.get_name(".meta.yml").unwrap();
        let meta_blob = repo.find_blob(meta_entry.id()).unwrap();
        assert_eq!(
            std::str::from_utf8(meta_blob.content()).unwrap(),
            "owner: testns\n"
        );

        let readme_entry = tree.get_name("README.md").unwrap();
        let readme_blob = repo.find_blob(readme_entry.id()).unwrap();
        assert_eq!(
            std::str::from_utf8(readme_blob.content()).unwrap(),
            "# castle\nBy testns\n"
        );
    }

    #[test]
    fn test_create_repo_unseeded() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "empty_area", false, None)
            .unwrap();
        assert!(mgr.repo_exists("testns", "empty_area"));

        // Verify it's a valid bare repo
        let repo_path = mgr.repo_path("testns", "empty_area");
        let repo = git2::Repository::open_bare(&repo_path).unwrap();
        assert!(repo.is_bare());

        // No branches should exist (no seed commit)
        assert!(repo.find_reference("refs/heads/main").is_err());
    }

    #[test]
    fn test_create_repo_duplicate() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        let result = mgr.create_repo("testns", "village", true, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_create_repo_invalid_name() {
        let (_dir, mgr) = setup();

        assert!(mgr.create_repo("Bad", "village", false, None).is_err());
        assert!(mgr.create_repo("testns", "my-repo", false, None).is_err());
        assert!(mgr.create_repo("", "village", false, None).is_err());
    }

    #[test]
    fn test_list_repos() {
        let (_dir, mgr) = setup();

        assert!(mgr.list_repos("testns").unwrap().is_empty());

        mgr.create_repo("testns", "beta", false, None).unwrap();
        mgr.create_repo("testns", "alpha", false, None).unwrap();

        let repos = mgr.list_repos("testns").unwrap();
        assert_eq!(repos, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_delete_repo() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", true, None).unwrap();
        assert!(mgr.repo_exists("testns", "village"));

        mgr.delete_repo("testns", "village").unwrap();
        assert!(!mgr.repo_exists("testns", "village"));

        assert!(!mgr.policy_path("testns", "village").exists());
    }

    #[test]
    fn test_delete_nonexistent_repo() {
        let (_dir, mgr) = setup();
        // Should not error on deleting a repo that doesn't exist
        mgr.delete_repo("testns", "nope").unwrap();
    }

    #[test]
    fn test_policy_default() {
        let (_dir, mgr) = setup();

        let policy = mgr.get_policy("testns", "village").unwrap();
        assert_eq!(policy.owner, "testns");
        assert!(policy.user_levels.is_empty());
    }

    #[test]
    fn test_policy_after_create() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", false, None).unwrap();
        let policy = mgr.get_policy("testns", "village").unwrap();
        assert_eq!(policy.owner, "testns");
        assert!(policy.user_levels.is_empty());
    }

    #[test]
    fn test_can_access_owner() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", false, None).unwrap();

        // Owner always has access
        assert!(mgr.can_access("testns", "testns", "village", &AccessLevel::ReadOnly));
        assert!(mgr.can_access("testns", "testns", "village", &AccessLevel::ReadWrite));
    }

    #[test]
    fn test_can_access_no_access() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", false, None).unwrap();

        assert!(!mgr.can_access("stranger", "testns", "village", &AccessLevel::ReadOnly));
        assert!(!mgr.can_access("stranger", "testns", "village", &AccessLevel::ReadWrite));
    }

    #[test]
    fn test_grant_and_check_access() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", false, None).unwrap();
        mgr.grant_access("testns", "village", "alice", AccessLevel::ReadOnly)
            .unwrap();

        assert!(mgr.can_access("alice", "testns", "village", &AccessLevel::ReadOnly));
        assert!(!mgr.can_access("alice", "testns", "village", &AccessLevel::ReadWrite));

        mgr.grant_access("testns", "village", "bob", AccessLevel::ReadWrite)
            .unwrap();
        assert!(mgr.can_access("bob", "testns", "village", &AccessLevel::ReadOnly));
        assert!(mgr.can_access("bob", "testns", "village", &AccessLevel::ReadWrite));
    }

    #[test]
    fn test_revoke_access() {
        let (_dir, mgr) = setup();

        mgr.create_repo("testns", "village", false, None).unwrap();
        mgr.grant_access("testns", "village", "alice", AccessLevel::ReadWrite)
            .unwrap();
        assert!(mgr.can_access("alice", "testns", "village", &AccessLevel::ReadWrite));

        mgr.revoke_access("testns", "village", "alice").unwrap();
        assert!(!mgr.can_access("alice", "testns", "village", &AccessLevel::ReadOnly));
    }

    #[test]
    fn test_repo_path() {
        let (_dir, mgr) = setup();
        let path = mgr.repo_path("testns", "village");
        assert!(path.ends_with("testns/village.git"));
    }

    #[test]
    fn test_create_repo_from_template_repo_creates_fresh_history() {
        let (_dir, mgr) = setup();
        let template_dir = TempDir::new().unwrap();
        let template_mgr = RepoManager::new(template_dir.path().to_path_buf());

        let mut template_files = HashMap::new();
        template_files.insert("README.md".into(), "# {{area_name}}\n".into());
        template_files.insert("mud.yaml".into(), "language: lpc\n".into());
        template_mgr
            .create_repo("system", "template_lpc", true, Some(&template_files))
            .unwrap();

        let checkout_dir = TempDir::new().unwrap();
        let workspace = crate::git::workspace::Workspace::new(
            checkout_dir.path().to_path_buf(),
            std::sync::Arc::new(template_mgr),
        );
        let template_worktree = workspace.checkout("system", "template_lpc").unwrap();

        mgr.create_repo_from_template_repo("alice", "wonder", &template_worktree, "main")
            .unwrap();

        let repo_path = mgr.repo_path("alice", "wonder");
        let repo = git2::Repository::open_bare(&repo_path).unwrap();
        let main_ref = repo.find_reference("refs/heads/main").unwrap();
        let develop_ref = repo.find_reference("refs/heads/develop").unwrap();
        assert_eq!(main_ref.target(), develop_ref.target());

        let commit = repo.find_commit(main_ref.target().unwrap()).unwrap();
        assert_eq!(commit.parent_count(), 0);

        let tree = commit.tree().unwrap();
        assert!(tree.get_name("README.md").is_some());
        assert!(tree.get_name("mud.yaml").is_some());
        assert!(tree.get_name(".meta.yml").is_some());
    }
}
