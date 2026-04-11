//! HTTP route definitions for the cisco-code server.
//!
//! REST API:
//! - POST   /api/v1/jobs              — Submit a new job
//! - GET    /api/v1/jobs              — List jobs
//! - GET    /api/v1/jobs/{id}         — Get job details
//! - DELETE /api/v1/jobs/{id}         — Cancel a job
//! - GET    /api/v1/jobs/{id}/stream  — SSE event stream
//! - GET    /api/v1/health            — Health check
//! - GET    /api/v1/version           — Server version info
//! - WS     /api/v1/ws/{session_id}   — WebSocket session


use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{delete, get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::jobs::{JobError, JobRequest, JobStatus};
use crate::state::AppState;
use crate::streaming;
use crate::websocket;

/// Build the complete axum router.
pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/jobs", post(create_job))
        .route("/jobs", get(list_jobs))
        .route("/jobs/{id}", get(get_job))
        .route("/jobs/{id}", delete(cancel_job))
        .route("/jobs/{id}/stream", get(stream_job))
        .route("/ws/{session_id}", get(websocket::ws_handler));

    Router::new()
        .nest("/api/v1", api)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Health & Version
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    uptime_secs: u64,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        uptime_secs: 0, // TODO: track actual uptime
    })
}

#[derive(Serialize)]
struct VersionResponse {
    version: String,
    agent: String,
}

async fn version(State(state): State<AppState>) -> Json<VersionResponse> {
    Json(VersionResponse {
        version: state.version.clone(),
        agent: "cisco-code".into(),
    })
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JobResponse {
    #[serde(flatten)]
    job: crate::jobs::Job,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

async fn create_job(
    State(state): State<AppState>,
    Json(request): Json<JobRequest>,
) -> Result<(StatusCode, Json<JobResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Extract execution params before submitting to job manager
    let prompt = request.prompt.clone();
    let session_id = request.session_id.clone();
    let model = request.model.clone();
    let max_turns = request.max_turns;

    match state.jobs.submit(request).await {
        Ok(job) => {
            // Spawn async execution via the executor
            state.executor.spawn(
                job.id.clone(),
                prompt,
                session_id,
                model,
                max_turns,
            );
            Ok((StatusCode::CREATED, Json(JobResponse { job })))
        }
        Err(JobError::QueueFull { max }) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse {
                error: format!("Job queue full (max {max} concurrent jobs)"),
                code: "queue_full".into(),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "internal".into(),
            }),
        )),
    }
}

#[derive(Deserialize)]
struct ListJobsQuery {
    status: Option<String>,
    limit: Option<usize>,
}

async fn list_jobs(
    State(state): State<AppState>,
    Query(query): Query<ListJobsQuery>,
) -> Json<Vec<crate::jobs::Job>> {
    let status_filter = query.status.as_deref().and_then(|s| match s {
        "queued" => Some(JobStatus::Queued),
        "running" => Some(JobStatus::Running),
        "completed" => Some(JobStatus::Completed),
        "failed" => Some(JobStatus::Failed),
        "cancelled" => Some(JobStatus::Cancelled),
        _ => None,
    });

    let mut jobs = state.jobs.list(status_filter.as_ref()).await;
    if let Some(limit) = query.limit {
        jobs.truncate(limit);
    }
    Json(jobs)
}

async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.jobs.get(&id).await {
        Some(job) => Ok(Json(JobResponse { job })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Job not found".into(),
                code: "not_found".into(),
            }),
        )),
    }
}

async fn cancel_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.jobs.cancel(&id).await {
        Ok(()) => {
            let job = state.jobs.get(&id).await.unwrap();
            Ok(Json(JobResponse { job }))
        }
        Err(JobError::NotFound) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Job not found".into(),
                code: "not_found".into(),
            }),
        )),
        Err(e) => Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "invalid_state".into(),
            }),
        )),
    }
}

async fn stream_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<
    axum::response::sse::Sse<impl futures::stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>>,
    (StatusCode, Json<ErrorResponse>),
> {
    match state.jobs.subscribe(&id, 256).await {
        Ok(rx) => Ok(streaming::stream_events(rx)),
        Err(JobError::NotFound) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Job not found".into(),
                code: "not_found".into(),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
                code: "internal".into(),
            }),
        )),
    }
}

/// Start the server with graceful shutdown on SIGTERM/Ctrl+C.
pub async fn serve(state: AppState, addr: &str) -> anyhow::Result<()> {
    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("cisco-code server listening on {addr}");

    let shutdown = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
        tracing::info!("Shutdown signal received, draining connections...");
    };

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, Method};
    use tower::ServiceExt;

    fn test_state() -> AppState {
        AppState::test("/tmp", "test-model")
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_version_endpoint() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["agent"], "cisco-code");
    }

    #[tokio::test]
    async fn test_create_job() {
        let app = build_router(test_state());
        let body = serde_json::json!({
            "prompt": "Fix the bug in main.rs"
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "queued");
        assert!(json["id"].is_string());
    }

    #[tokio::test]
    async fn test_list_jobs_empty() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/jobs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_get_job_not_found() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/jobs/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_cancel_job_not_found() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/v1/jobs/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_create_and_get_job() {
        let state = test_state();

        // Create job
        let app = build_router(state.clone());
        let body = serde_json::json!({
            "prompt": "hello",
            "model": "test-model"
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/jobs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        let job_id = json["id"].as_str().unwrap().to_string();

        // Get job
        let app = build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/jobs/{job_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["request"]["prompt"], "hello");
    }

    #[tokio::test]
    async fn test_list_jobs_with_status_filter() {
        let state = test_state();

        // Create two jobs
        let _ = state.jobs.submit(JobRequest {
            prompt: "a".into(),
            session_id: None,
            model: None,
            max_turns: None,
            cwd: None,
        }).await.unwrap();

        let job2 = state.jobs.submit(JobRequest {
            prompt: "b".into(),
            session_id: None,
            model: None,
            max_turns: None,
            cwd: None,
        }).await.unwrap();

        state.jobs.mark_running(&job2.id).await.unwrap();

        // Filter for running
        let app = build_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/jobs?status=running")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);
    }
}
