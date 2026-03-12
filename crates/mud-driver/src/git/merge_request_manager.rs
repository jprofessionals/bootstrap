use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tracing::info;

use crate::git::repo_manager::RepoManager;
use crate::git::workspace::Workspace;
use crate::persistence::merge_request_store::{
    CreateMrParams, MergeRequest, MergeRequestStore, MrApproval,
};

/// Policy controlling merge request workflow rules.
///
/// When `main_protected` is true, direct pushes to main are rejected
/// (enforced at the git hook level). The `required_approvals` field sets
/// the minimum number of approvals needed before a merge request can be
/// merged.
#[derive(Default)]
pub struct ReviewPolicy {
    pub main_protected: bool,
    pub required_approvals: u32,
}

/// Callback invoked after a merge completes, receiving `(namespace, area_name)`.
type OnMergeCallback = Box<dyn Fn(&str, &str) + Send + Sync>;

/// Orchestrates the merge request lifecycle: creation, approval, policy
/// enforcement, and merge execution using git2.
///
/// The manager delegates persistence to [`MergeRequestStore`] and uses
/// [`RepoManager`] / [`Workspace`] for git operations. An optional
/// `on_merge` callback is invoked after a successful merge to allow the
/// caller to trigger area reloads or other side effects.
pub struct MergeRequestManager {
    store: MergeRequestStore,
    repo_manager: Arc<RepoManager>,
    workspace: Arc<Workspace>,
    policy: ReviewPolicy,
    on_merge: Option<OnMergeCallback>,
}

impl MergeRequestManager {
    pub fn new(
        store: MergeRequestStore,
        repo_manager: Arc<RepoManager>,
        workspace: Arc<Workspace>,
        policy: ReviewPolicy,
    ) -> Self {
        Self {
            store,
            repo_manager,
            workspace,
            policy,
            on_merge: None,
        }
    }

    /// Register a callback that is invoked after a merge completes.
    ///
    /// The callback receives `(namespace, area_name)` so the caller can
    /// trigger an area reload or other post-merge action.
    pub fn set_on_merge(&mut self, callback: OnMergeCallback) {
        self.on_merge = Some(callback);
    }

    /// Create a new merge request in "open" state.
    pub async fn create_merge_request(&self, params: CreateMrParams) -> Result<MergeRequest> {
        let mr = self
            .store
            .create(params)
            .await
            .context("creating merge request")?;
        info!(
            mr_id = mr.id,
            ns = %mr.namespace,
            area = %mr.area_name,
            title = %mr.title,
            "merge request created"
        );
        Ok(mr)
    }

    /// Add an approval to a merge request.
    ///
    /// The MR must be in "open" state. If the required approval count is
    /// met after this approval, the MR state is updated to "approved".
    pub async fn add_approval(
        &self,
        mr_id: i32,
        approver: &str,
        comment: Option<&str>,
    ) -> Result<MrApproval> {
        let mr = self
            .store
            .find(mr_id)
            .await?
            .context("merge request not found")?;

        if mr.state != "open" {
            bail!(
                "cannot approve MR #{}: state is '{}', expected 'open'",
                mr_id,
                mr.state
            );
        }

        let approval = self
            .store
            .add_approval(mr_id, approver, comment)
            .await
            .context("adding approval")?;

        info!(mr_id, approver, "approval added");

        // Auto-transition to "approved" if threshold met
        let count = self.store.approval_count(mr_id).await?;
        if count >= self.policy.required_approvals as i64 && self.policy.required_approvals > 0 {
            self.store.update_state(mr_id, "approved").await?;
            info!(mr_id, "merge request auto-approved (threshold met)");
        }

        Ok(approval)
    }

    /// Execute the merge for a merge request.
    ///
    /// The MR must be in "open" or "approved" state. The required approval
    /// count (from the review policy) must be met. The merge is performed
    /// on the bare repository using git2: source and target branch commits
    /// are merged, conflicts cause a failure, and on success a merge commit
    /// is created updating the target branch ref.
    pub async fn execute_merge(&self, mr_id: i32) -> Result<MergeRequest> {
        // 1. Load MR and verify state
        let mr = self
            .store
            .find(mr_id)
            .await?
            .context("merge request not found")?;

        if mr.state != "open" && mr.state != "approved" {
            bail!(
                "cannot merge MR #{}: state is '{}', expected 'open' or 'approved'",
                mr_id,
                mr.state
            );
        }

        // 2. Check approval threshold
        let approvals = self.store.approval_count(mr_id).await?;
        if approvals < self.policy.required_approvals as i64 {
            bail!(
                "cannot merge MR #{}: has {} approvals, requires {}",
                mr_id,
                approvals,
                self.policy.required_approvals
            );
        }

        // 3-8. Perform the git merge on the bare repo
        let bare_path = self.repo_manager.repo_path(&mr.namespace, &mr.area_name);
        let source_branch = mr.source_branch.clone();
        let target_branch = mr.target_branch.clone();
        let title = mr.title.clone();

        // Run the blocking git2 operations in a spawn_blocking task
        tokio::task::spawn_blocking(move || -> Result<()> {
            let repo = git2::Repository::open_bare(&bare_path)
                .context("opening bare repository for merge")?;

            // 4. Lookup source and target branch refs
            let source_ref = repo
                .find_reference(&format!("refs/heads/{}", source_branch))
                .with_context(|| format!("source branch '{}' not found", source_branch))?;
            let target_ref = repo
                .find_reference(&format!("refs/heads/{}", target_branch))
                .with_context(|| format!("target branch '{}' not found", target_branch))?;

            let source_commit = source_ref
                .peel_to_commit()
                .context("resolving source commit")?;
            let target_commit = target_ref
                .peel_to_commit()
                .context("resolving target commit")?;

            // 5. Create merge index
            let mut merge_index = repo
                .merge_commits(&target_commit, &source_commit, None)
                .context("performing merge")?;

            // 6. Check for conflicts
            if merge_index.has_conflicts() {
                bail!(
                    "merge conflict between '{}' and '{}'",
                    source_branch,
                    target_branch
                );
            }

            // 7. Write tree from merge index
            let tree_oid = merge_index
                .write_tree_to(&repo)
                .context("writing merge tree")?;
            let tree = repo.find_tree(tree_oid)?;

            // 8. Create merge commit with both parents
            let sig = git2::Signature::now("MUD Driver", "mud@localhost")?;
            let message = format!(
                "Merge '{}' into {}: {}",
                source_branch, target_branch, title
            );

            repo.commit(
                Some(&format!("refs/heads/{}", target_branch)),
                &sig,
                &sig,
                &message,
                &tree,
                &[&target_commit, &source_commit],
            )
            .context("creating merge commit")?;

            Ok(())
        })
        .await
        .context("spawn_blocking merge task panicked")??;

        // 9. Update MR state to "merged"
        self.store.update_state(mr_id, "merged").await?;
        info!(mr_id, "merge request merged");

        // 10. Call on_merge callback if set
        if let Some(ref callback) = self.on_merge {
            callback(&mr.namespace, &mr.area_name);
        }

        // Pull the target branch working copy so it reflects the merge
        if let Err(e) = self
            .workspace
            .pull(&mr.namespace, &mr.area_name, &mr.target_branch)
        {
            tracing::warn!(
                mr_id,
                error = %e,
                "failed to pull workspace after merge"
            );
        }

        // Return updated MR
        self.store
            .find(mr_id)
            .await?
            .context("merge request not found after merge")
    }

    /// Reject a merge request.
    pub async fn reject(&self, mr_id: i32, _reviewer: &str, _reason: &str) -> Result<()> {
        let mr = self
            .store
            .find(mr_id)
            .await?
            .context("merge request not found")?;

        if mr.state != "open" && mr.state != "approved" {
            bail!(
                "cannot reject MR #{}: state is '{}', expected 'open' or 'approved'",
                mr_id,
                mr.state
            );
        }

        self.store.update_state(mr_id, "rejected").await?;
        info!(mr_id, "merge request rejected");
        Ok(())
    }

    /// Close a merge request without merging.
    pub async fn close(&self, mr_id: i32) -> Result<()> {
        let mr = self
            .store
            .find(mr_id)
            .await?
            .context("merge request not found")?;

        if mr.state != "open" && mr.state != "approved" {
            bail!(
                "cannot close MR #{}: state is '{}', expected 'open' or 'approved'",
                mr_id,
                mr.state
            );
        }

        self.store.update_state(mr_id, "closed").await?;
        info!(mr_id, "merge request closed");
        Ok(())
    }

    /// Reopen a previously closed or rejected merge request.
    pub async fn reopen(&self, mr_id: i32) -> Result<()> {
        let mr = self
            .store
            .find(mr_id)
            .await?
            .context("merge request not found")?;

        if mr.state != "closed" && mr.state != "rejected" {
            bail!(
                "cannot reopen MR #{}: state is '{}', expected 'closed' or 'rejected'",
                mr_id,
                mr.state
            );
        }

        self.store.update_state(mr_id, "open").await?;
        info!(mr_id, "merge request reopened");
        Ok(())
    }

    /// Look up a merge request by ID.
    pub async fn get(&self, id: i32) -> Result<Option<MergeRequest>> {
        self.store.find(id).await
    }

    /// List merge requests for a namespace/area, optionally filtered by state.
    pub async fn list(
        &self,
        ns: &str,
        area: &str,
        state: Option<&str>,
    ) -> Result<Vec<MergeRequest>> {
        self.store.list(ns, area, state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_policy_default() {
        let policy = ReviewPolicy::default();
        assert!(!policy.main_protected);
        assert_eq!(policy.required_approvals, 0);
    }

    #[test]
    fn review_policy_custom() {
        let policy = ReviewPolicy {
            main_protected: true,
            required_approvals: 2,
        };
        assert!(policy.main_protected);
        assert_eq!(policy.required_approvals, 2);
    }

    #[test]
    fn review_policy_no_approvals_needed() {
        let policy = ReviewPolicy {
            main_protected: true,
            required_approvals: 0,
        };
        assert!(policy.main_protected);
        assert_eq!(policy.required_approvals, 0);
    }

    #[test]
    fn review_policy_high_approval_threshold() {
        let policy = ReviewPolicy {
            main_protected: true,
            required_approvals: 10,
        };
        assert_eq!(policy.required_approvals, 10);
    }
}
