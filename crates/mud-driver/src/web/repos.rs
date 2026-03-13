use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::git::repo_manager::RepoManager;
use crate::git::workspace::Workspace;
use crate::server::{AreaTemplates, TemplateRegistry};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ReposState {
    repo_manager: Arc<RepoManager>,
    workspace: Arc<Workspace>,
    area_templates: AreaTemplates,
    template_registry: TemplateRegistry,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateRepoRequest {
    namespace: String,
    name: String,
    #[serde(default)]
    template: Option<String>,
}

#[derive(Debug, Serialize)]
struct TemplateInfo {
    name: String,
    file_count: usize,
    display_name: Option<String>,
    description: Option<String>,
    language: Option<String>,
    framework: Option<String>,
}

#[derive(Debug, Serialize)]
struct TemplatesResponse {
    templates: Vec<TemplateInfo>,
}

#[derive(Debug, Serialize)]
struct CreateRepoResponse {
    status: String,
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn repos_routes(
    repo_manager: Arc<RepoManager>,
    workspace: Arc<Workspace>,
    area_templates: AreaTemplates,
    template_registry: TemplateRegistry,
) -> Router {
    let state = ReposState {
        repo_manager,
        workspace,
        area_templates,
        template_registry,
    };

    Router::new()
        .route("/templates", get(list_templates_handler))
        .route("/create", post(create_repo_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// List all registered area templates.
async fn list_templates_handler(State(state): State<ReposState>) -> Response {
    let system_templates = state.template_registry.read().await;
    let mut list: Vec<TemplateInfo> = if system_templates.is_empty() {
        let templates = state.area_templates.read().await;
        templates
            .iter()
            .map(|(name, files)| TemplateInfo {
                name: name.clone(),
                file_count: files.len(),
                display_name: None,
                description: None,
                language: None,
                framework: None,
            })
            .collect()
    } else {
        system_templates
            .values()
            .map(|template| TemplateInfo {
                name: template.name.clone(),
                file_count: count_template_files(&template.path),
                display_name: template.metadata.display_name.clone(),
                description: template.metadata.description.clone(),
                language: Some(template.metadata.language.clone()),
                framework: template.metadata.framework.clone(),
            })
            .collect()
    };
    list.sort_by(|a, b| a.name.cmp(&b.name));

    Json(TemplatesResponse { templates: list }).into_response()
}

/// Create a new repository, optionally from a named template.
async fn create_repo_handler(
    State(state): State<ReposState>,
    Json(body): Json<CreateRepoRequest>,
) -> Response {
    if body.namespace.is_empty() || body.name.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "namespace and name are required");
    }

    let system_templates = state.template_registry.read().await;
    if !system_templates.is_empty() {
        let template = body
            .template
            .as_ref()
            .and_then(|name| system_templates.get(name))
            .or_else(|| system_templates.get("default"))
            .or_else(|| system_templates.values().next());

        let Some(template) = template else {
            return error_response(StatusCode::BAD_REQUEST, "no templates available");
        };

        if let Err(e) = state.repo_manager.create_repo_from_template_repo(
            &body.namespace,
            &body.name,
            &template.path,
            "main",
        ) {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create repo from template: {e}"),
            );
        }
    } else {
        let templates = state.area_templates.read().await;
        let template = body
            .template
            .as_ref()
            .and_then(|name| templates.get(name))
            .or_else(|| templates.get("default"))
            .or_else(|| templates.values().next());

        if let Err(e) = state
            .repo_manager
            .create_repo(&body.namespace, &body.name, true, template)
        {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create repo: {e}"),
            );
        }
    }

    // Check out the workspace so files are immediately accessible.
    if let Err(e) = state.workspace.checkout(&body.namespace, &body.name) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("repo created but checkout failed: {e}"),
        );
    }

    Json(CreateRepoResponse {
        status: "ok".into(),
    })
    .into_response()
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn count_template_files(root: &std::path::Path) -> usize {
    fn visit(dir: &std::path::Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };

        entries
            .flatten()
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    visit(&path)
                } else if path.is_file() {
                    1
                } else {
                    0
                }
            })
            .sum()
    }

    visit(root)
}
