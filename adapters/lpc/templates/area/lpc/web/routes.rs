use mud_adapter_sdk::prelude::*;
use axum::{Router, Json, routing::get};

#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("web/routes");
    registrar.set_type(ModuleType::Web);
    registrar.register_router(router);
}

fn router() -> Router<AppState> {
    Router::new()
        .route("/api/status", get(status))
}

async fn status() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}
