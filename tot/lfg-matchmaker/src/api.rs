use crate::queue::Queue;
use crate::types::{QueueEntry, Role};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub struct AppState {
    pub queue: Queue,
}

#[derive(Deserialize)]
pub struct QueueReq {
    pub guid: u64,
    pub role: String,
    pub dungeon_id: u32,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/queue", post(enqueue).get(list))
        .route("/queue/:guid", delete(dequeue))
        .with_state(state)
}

async fn enqueue(
    State(s): State<Arc<AppState>>,
    Json(req): Json<QueueReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let role = Role::parse(&req.role).ok_or((StatusCode::BAD_REQUEST, format!("bad role: {}", req.role)))?;
    s.queue.upsert(QueueEntry { guid: req.guid, role, dungeon_id: req.dungeon_id });
    Ok(Json(serde_json::json!({ "queued": true, "guid": req.guid, "depth": s.queue.len() })))
}

async fn dequeue(State(s): State<Arc<AppState>>, Path(guid): Path<u64>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "removed": s.queue.remove(guid) }))
}

async fn list(State(s): State<Arc<AppState>>) -> Json<Vec<QueueEntry>> {
    Json(s.queue.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState { queue: Queue::new() })
    }

    async fn spawn(state: Arc<AppState>) -> String {
        let app = router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn enqueue_then_list_then_delete() {
        let state = test_state();
        let base = spawn(state.clone()).await;
        let client = reqwest::Client::new();

        let r = client
            .post(format!("{base}/queue"))
            .json(&serde_json::json!({"guid": 42, "role": "tank", "dungeon_id": 36}))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
        assert_eq!(state.queue.len(), 1);

        let listed: Vec<QueueEntry> =
            client.get(format!("{base}/queue")).send().await.unwrap().json().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].guid, 42);

        let r = client.delete(format!("{base}/queue/42")).send().await.unwrap();
        assert!(r.status().is_success());
        assert_eq!(state.queue.len(), 0);
    }

    #[tokio::test]
    async fn bad_role_is_rejected() {
        let base = spawn(test_state()).await;
        let r = reqwest::Client::new()
            .post(format!("{base}/queue"))
            .json(&serde_json::json!({"guid": 1, "role": "wizard", "dungeon_id": 36}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
    }
}
