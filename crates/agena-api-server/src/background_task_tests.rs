use agena_application::Application;
use agena_runtime::{
    RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOrigin,
    RuntimeBootstrapRequest, RuntimeControlService, bootstrap_application_services,
};
use axum::{body::Body, body::to_bytes, http::Request, http::StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn settings_reload_after_shutdown_returns_unavailability_without_a_completion_report() {
    let workspace = tempfile::tempdir().unwrap();
    let config_path = workspace.path().join("global.json");
    let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
        workspace_root: Some(workspace.path().to_path_buf()),
        config_path: Some(config_path.clone()),
        database_url: Some("sqlite::memory:".into()),
        scheduler_database_url: Some("sqlite::memory:".into()),
        initialize_schema: true,
        ..RuntimeBootstrapRequest::default()
    })
    .await
    .unwrap();
    let services = runtime.application_services();
    let status = services.status.clone();
    let generation = status.runtime_status().await.generation;
    let application = Application::from_composed_runtime_services(services).unwrap();
    let app = super::router(super::AppState::from_application(application.clone()));
    runtime.shutdown();
    let response = app
        .oneshot(
            Request::put("/api/v1/settings")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "path":"ui.locale", "value":"zh-CN", "dry_run":false,
                        "validate":true, "reload":true,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let http_status = response.status();
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    // The file edit precedes reload. Rejection must not imply that it was
    // rolled back or manufacture a completed runtime generation.
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
    assert_eq!(saved["ui"]["locale"], "zh-CN");
    assert_eq!(status.runtime_status().await.generation, generation);
    assert_eq!(http_status, StatusCode::SERVICE_UNAVAILABLE);
    let error: agena_api::error::ApiError = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        error.problem.category,
        agena_failure::FailureCategory::DependencyUnavailable
    );
    assert_eq!(error.problem.retry, agena_failure::RetryDirective::Backoff);
    assert!(
        serde_json::from_slice::<serde_json::Value>(&body)
            .unwrap()
            .get("reload")
            .is_none()
    );

    // Application clients edit through separate set/delete use cases. Both
    // must preserve the same stopped-runtime classification as the REST path.
    for error in [
        application
            .set_config_setting("ui.locale", serde_json::json!("en-US"))
            .await
            .unwrap_err(),
        application
            .delete_config_setting("ui.locale")
            .await
            .unwrap_err(),
    ] {
        assert_eq!(
            error.failure.category,
            agena_failure::FailureCategory::DependencyUnavailable
        );
        assert_eq!(error.failure.retry, agena_failure::RetryDirective::Backoff);
        assert!(
            error
                .diagnostic_message()
                .unwrap()
                .contains("after config change")
        );
    }
    assert_eq!(status.runtime_status().await.generation, generation);
}

async fn wait_for_tasks(control: &dyn RuntimeControlService) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while control
            .background_tasks()
            .iter()
            .any(|task| task.is_running())
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled background workers must reach a terminal state");
}

#[tokio::test]
async fn reload_and_catalog_routes_preserve_capacity_and_shutdown_failures() {
    let workspace = tempfile::tempdir().unwrap();
    let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
        workspace_root: Some(workspace.path().to_path_buf()),
        config_path: Some(workspace.path().join("global.json")),
        database_url: Some("sqlite::memory:".into()),
        scheduler_database_url: Some("sqlite::memory:".into()),
        initialize_schema: true,
        ..RuntimeBootstrapRequest::default()
    })
    .await
    .unwrap();
    let services = runtime.application_services();
    let control = services.control.clone();
    for task in control
        .background_tasks()
        .into_iter()
        .filter(|task| task.is_running())
    {
        control.cancel_background_task(&task.id).unwrap();
    }
    wait_for_tasks(control.as_ref()).await;
    let application = Application::from_composed_runtime_services(services).unwrap();
    let app = super::router(super::AppState::from_application(application));
    for _ in 0..64 {
        control
            .start_background_task(
                RuntimeBackgroundTaskKind::RuntimeReload,
                RuntimeBackgroundTaskOrigin::User,
                "held API capacity fixture".into(),
                None,
                true,
                Box::new(|_| Box::pin(std::future::pending())),
            )
            .unwrap();
    }
    assert!(matches!(
        control.start_background_task(
            RuntimeBackgroundTaskKind::RuntimeReload,
            RuntimeBackgroundTaskOrigin::User,
            "rejected API capacity fixture".into(),
            None,
            true,
            Box::new(|_| Box::pin(std::future::pending())),
        ),
        Err(RuntimeBackgroundTaskControlError::Capacity { limit: 64 })
    ));

    let mut responses = Vec::new();
    for shutting_down in [false, true] {
        if shutting_down {
            runtime.shutdown();
            wait_for_tasks(control.as_ref()).await;
        }
        for path in ["/api/v1/runtime/reload", "/api/v1/model-catalog/refresh"] {
            let response = app
                .clone()
                .oneshot(Request::post(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            responses.push((path, shutting_down, status, body));
        }
        assert_eq!(
            control
                .background_tasks()
                .iter()
                .filter(|task| task.is_running())
                .count(),
            if shutting_down { 0 } else { 64 },
            "rejected requests must not create additional background workers"
        );
    }
    // Check after shutdown so a failed response assertion cannot retain the
    // held fixtures or their runtime ownership beyond this test.
    for (path, shutting_down, status, body) in responses {
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path}, shutdown={shutting_down}"
        );
        let error: agena_api::error::ApiError = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            error.problem.category,
            agena_failure::FailureCategory::DependencyUnavailable
        );
        assert_eq!(error.problem.retry, agena_failure::RetryDirective::Backoff);
        assert!(
            serde_json::from_slice::<serde_json::Value>(&body)
                .unwrap()
                .get("diagnostic")
                .is_none()
        );
    }
}
