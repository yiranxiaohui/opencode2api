use axum::Router;
use axum::routing::{any, delete, get, post, put};
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

pub mod auth;
pub mod client_keys;
pub mod import_export;
pub mod keys;
pub mod logs;
pub mod messages;
pub mod models;
pub mod proxies;
pub mod proxy;

pub fn build_router(state: AppState) -> Router {
    let dist = state.web_dist.clone();
    // Serve built frontend; fall back to index.html for SPA deep links.
    // Use `.fallback` (not `.not_found_service`) so deep-link routes get a
    // proper 200 — not_found_service force-rewrites the status to 404.
    //
    // NOTE: no CORS layer on purpose. The UI is same-origin (axum serves the
    // built frontend; Vite proxies in dev), so CORS headers are unnecessary —
    // and a permissive layer would let ANY website the user visits fire the
    // proxy / read the vault's data via the browser. Bind stays on 127.0.0.1.
    let serve = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));

    let api = Router::new()
        .route("/status", get(auth::status))
        .route("/auth/setup", post(auth::setup))
        .route("/auth/unlock", post(auth::unlock))
        .route("/auth/change-password", post(auth::change_password))
        .route(
            "/client-keys",
            get(client_keys::list).post(client_keys::create),
        )
        .route("/client-keys/{id}", delete(client_keys::delete))
        .route("/logs", get(logs::list).delete(logs::clear))
        .route("/logs/stats", get(logs::stats))
        .route("/chat/{*path}", any(proxy::chat_proxy))
        .route("/keys", get(keys::list).post(keys::create))
        .route(
            "/keys/{id}",
            get(keys::get_key).put(keys::update).delete(keys::delete),
        )
        .route("/keys/{id}/test", post(keys::test))
        .route("/keys/{id}/set-default", post(keys::set_default))
        .route("/proxies", get(proxies::list).post(proxies::create))
        .route(
            "/proxies/{id}",
            put(proxies::update).delete(proxies::delete),
        )
        .route("/export", get(import_export::export))
        .route("/import", post(import_export::import));

    Router::new()
        .nest("/api", api)
        // Wildcard matches `/v1` too (zero-or-more segments).
        .route("/v1/models", get(models::models))
        .route("/v1/messages", post(messages::messages))
        .route("/v1/{*path}", any(proxy::proxy))
        .fallback_service(serve)
        .with_state(state)
}
