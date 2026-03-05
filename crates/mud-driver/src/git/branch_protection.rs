use std::path::Path;
use anyhow::Result;
use std::os::unix::fs::PermissionsExt;

const PRE_RECEIVE_HOOK: &str = r#"#!/bin/sh
while read oldrev newrev refname; do
  if [ "$refname" = "refs/heads/main" ]; then
    echo "ERROR: Direct pushes to main are not allowed."
    echo "Push to a feature branch and create a merge request."
    exit 1
  fi
done
"#;

/// Install a pre-receive hook that rejects direct pushes to main.
pub fn install_branch_protection(repo_path: &Path) -> Result<()> {
    let hooks_dir = repo_path.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join("pre-receive");
    std::fs::write(&hook_path, PRE_RECEIVE_HOOK)?;
    // Make executable
    let mut perms = std::fs::metadata(&hook_path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&hook_path, perms)?;
    Ok(())
}

/// Remove the pre-receive hook.
pub fn remove_branch_protection(repo_path: &Path) -> Result<()> {
    let hook_path = repo_path.join("hooks").join("pre-receive");
    if hook_path.exists() {
        std::fs::remove_file(&hook_path)?;
    }
    Ok(())
}

/// Check if branch protection is installed.
pub fn is_protected(repo_path: &Path) -> bool {
    repo_path.join("hooks").join("pre-receive").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_creates_hook_file() {
        let dir = tempfile::tempdir().unwrap();
        install_branch_protection(dir.path()).unwrap();
        assert!(dir.path().join("hooks/pre-receive").exists());
    }

    #[test]
    fn install_makes_hook_executable() {
        let dir = tempfile::tempdir().unwrap();
        install_branch_protection(dir.path()).unwrap();
        let metadata = std::fs::metadata(dir.path().join("hooks/pre-receive")).unwrap();
        let mode = metadata.permissions().mode();
        assert_eq!(mode & 0o755, 0o755);
    }

    #[test]
    fn hook_content_rejects_main() {
        let dir = tempfile::tempdir().unwrap();
        install_branch_protection(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("hooks/pre-receive")).unwrap();
        assert!(content.contains("refs/heads/main"));
        assert!(content.contains("exit 1"));
    }

    #[test]
    fn hook_content_is_shell_script() {
        let dir = tempfile::tempdir().unwrap();
        install_branch_protection(dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("hooks/pre-receive")).unwrap();
        assert!(content.starts_with("#!/bin/sh"));
    }

    #[test]
    fn is_protected_true_after_install() {
        let dir = tempfile::tempdir().unwrap();
        install_branch_protection(dir.path()).unwrap();
        assert!(is_protected(dir.path()));
    }

    #[test]
    fn is_protected_false_initially() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_protected(dir.path()));
    }

    #[test]
    fn remove_deletes_hook() {
        let dir = tempfile::tempdir().unwrap();
        install_branch_protection(dir.path()).unwrap();
        assert!(is_protected(dir.path()));
        remove_branch_protection(dir.path()).unwrap();
        assert!(!is_protected(dir.path()));
    }

    #[test]
    fn remove_noop_when_no_hook() {
        let dir = tempfile::tempdir().unwrap();
        // Should not error even if hook doesn't exist
        remove_branch_protection(dir.path()).unwrap();
    }

    #[test]
    fn install_creates_hooks_dir() {
        let dir = tempfile::tempdir().unwrap();
        // hooks dir doesn't exist yet
        assert!(!dir.path().join("hooks").exists());
        install_branch_protection(dir.path()).unwrap();
        assert!(dir.path().join("hooks").is_dir());
    }

    #[test]
    fn install_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        install_branch_protection(dir.path()).unwrap();
        install_branch_protection(dir.path()).unwrap(); // second call should succeed
        assert!(is_protected(dir.path()));
    }
}
