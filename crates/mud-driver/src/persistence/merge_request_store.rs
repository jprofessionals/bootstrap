use sqlx::PgPool;
use anyhow::{Result, Context};
use chrono::{DateTime, Utc};
use sqlx::Row;

#[derive(Debug, Clone)]
pub struct MergeRequest {
    pub id: i32,
    pub namespace: String,
    pub area_name: String,
    pub title: String,
    pub description: Option<String>,
    pub author: String,
    pub state: String,
    pub source_branch: String,
    pub target_branch: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct MrApproval {
    pub id: i32,
    pub merge_request_id: i32,
    pub approver: String,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub struct CreateMrParams {
    pub namespace: String,
    pub area_name: String,
    pub author: String,
    pub title: String,
    pub description: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
}

impl Default for CreateMrParams {
    fn default() -> Self {
        Self {
            namespace: String::new(),
            area_name: String::new(),
            author: String::new(),
            title: String::new(),
            description: None,
            source_branch: "develop".to_string(),
            target_branch: "main".to_string(),
        }
    }
}

pub struct MergeRequestStore {
    pool: PgPool,
}

impl MergeRequestStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, params: CreateMrParams) -> Result<MergeRequest> {
        let row = sqlx::query(
            "INSERT INTO merge_requests (namespace, area_name, author, title, description, source_branch, target_branch) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING id, namespace, area_name, title, description, author, state, source_branch, target_branch, created_at, updated_at",
        )
        .bind(&params.namespace)
        .bind(&params.area_name)
        .bind(&params.author)
        .bind(&params.title)
        .bind(&params.description)
        .bind(&params.source_branch)
        .bind(&params.target_branch)
        .fetch_one(&self.pool)
        .await
        .context("creating merge request")?;

        Ok(self.row_to_mr(&row))
    }

    pub async fn find(&self, id: i32) -> Result<Option<MergeRequest>> {
        let row = sqlx::query(
            "SELECT id, namespace, area_name, title, description, author, state, source_branch, target_branch, created_at, updated_at \
             FROM merge_requests WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_ref().map(|r| self.row_to_mr(r)))
    }

    pub async fn list(
        &self,
        ns: &str,
        area: &str,
        state: Option<&str>,
    ) -> Result<Vec<MergeRequest>> {
        let rows = if let Some(state) = state {
            sqlx::query(
                "SELECT id, namespace, area_name, title, description, author, state, source_branch, target_branch, created_at, updated_at \
                 FROM merge_requests WHERE namespace = $1 AND area_name = $2 AND state = $3 \
                 ORDER BY created_at DESC",
            )
            .bind(ns)
            .bind(area)
            .bind(state)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, namespace, area_name, title, description, author, state, source_branch, target_branch, created_at, updated_at \
                 FROM merge_requests WHERE namespace = $1 AND area_name = $2 \
                 ORDER BY created_at DESC",
            )
            .bind(ns)
            .bind(area)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows.iter().map(|r| self.row_to_mr(r)).collect())
    }

    pub async fn update_state(&self, id: i32, state: &str) -> Result<()> {
        sqlx::query(
            "UPDATE merge_requests SET state = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2",
        )
        .bind(state)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_approval(
        &self,
        mr_id: i32,
        approver: &str,
        comment: Option<&str>,
    ) -> Result<MrApproval> {
        let row = sqlx::query(
            "INSERT INTO merge_request_approvals (merge_request_id, approver, comment) \
             VALUES ($1, $2, $3) RETURNING id, merge_request_id, approver, comment, created_at",
        )
        .bind(mr_id)
        .bind(approver)
        .bind(comment)
        .fetch_one(&self.pool)
        .await
        .context("adding approval")?;

        Ok(MrApproval {
            id: row.get("id"),
            merge_request_id: row.get("merge_request_id"),
            approver: row.get("approver"),
            comment: row.get("comment"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn approvals(&self, mr_id: i32) -> Result<Vec<MrApproval>> {
        let rows = sqlx::query(
            "SELECT id, merge_request_id, approver, comment, created_at \
             FROM merge_request_approvals WHERE merge_request_id = $1 ORDER BY created_at",
        )
        .bind(mr_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| MrApproval {
                id: r.get("id"),
                merge_request_id: r.get("merge_request_id"),
                approver: r.get("approver"),
                comment: r.get("comment"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    pub async fn approval_count(&self, mr_id: i32) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM merge_request_approvals WHERE merge_request_id = $1",
        )
        .bind(mr_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    fn row_to_mr(&self, row: &sqlx::postgres::PgRow) -> MergeRequest {
        MergeRequest {
            id: row.get("id"),
            namespace: row.get("namespace"),
            area_name: row.get("area_name"),
            title: row.get("title"),
            description: row.get("description"),
            author: row.get("author"),
            state: row.get("state"),
            source_branch: row.get("source_branch"),
            target_branch: row.get("target_branch"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_mr_params_default() {
        let params = CreateMrParams::default();
        assert_eq!(params.namespace, "");
        assert_eq!(params.area_name, "");
        assert_eq!(params.author, "");
        assert_eq!(params.title, "");
        assert!(params.description.is_none());
        assert_eq!(params.source_branch, "develop");
        assert_eq!(params.target_branch, "main");
    }

    #[test]
    fn merge_request_clone() {
        let mr = MergeRequest {
            id: 1,
            namespace: "test".into(),
            area_name: "arena".into(),
            title: "Fix combat".into(),
            description: Some("Detailed fix".into()),
            author: "alice".into(),
            state: "open".into(),
            source_branch: "develop".into(),
            target_branch: "main".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let cloned = mr.clone();
        assert_eq!(cloned.id, 1);
        assert_eq!(cloned.namespace, "test");
        assert_eq!(cloned.title, "Fix combat");
        assert_eq!(cloned.description, Some("Detailed fix".into()));
        assert_eq!(cloned.state, "open");
    }

    #[test]
    fn merge_request_debug() {
        let mr = MergeRequest {
            id: 42,
            namespace: "ns".into(),
            area_name: "area".into(),
            title: "Title".into(),
            description: None,
            author: "bob".into(),
            state: "merged".into(),
            source_branch: "develop".into(),
            target_branch: "main".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let debug = format!("{:?}", mr);
        assert!(debug.contains("42"));
        assert!(debug.contains("merged"));
    }

    #[test]
    fn mr_approval_clone() {
        let approval = MrApproval {
            id: 1,
            merge_request_id: 10,
            approver: "charlie".into(),
            comment: Some("LGTM".into()),
            created_at: Utc::now(),
        };
        let cloned = approval.clone();
        assert_eq!(cloned.id, 1);
        assert_eq!(cloned.merge_request_id, 10);
        assert_eq!(cloned.approver, "charlie");
        assert_eq!(cloned.comment, Some("LGTM".into()));
    }

    #[test]
    fn mr_approval_without_comment() {
        let approval = MrApproval {
            id: 2,
            merge_request_id: 10,
            approver: "dave".into(),
            comment: None,
            created_at: Utc::now(),
        };
        assert!(approval.comment.is_none());
    }
}
