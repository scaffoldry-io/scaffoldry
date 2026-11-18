//! Phase 11 (one image): the server serves the built web files when SCAFFOLDRY_WEB_DIR is set.
//! The variable is process-wide, so this file is its own test binary.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app.clone().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn web_files_are_public_and_api_stays_guarded() {
    let _db = common::TestDb::new();
    let dir = std::env::temp_dir().join(format!("scaffoldry-web-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>scaffoldry-web</html>").unwrap();
    std::fs::write(dir.join("app.js"), "console.log(1)").unwrap();
    std::env::set_var("SCAFFOLDRY_WEB_DIR", &dir);
    let app = scaffoldry_server::build_app().expect("router");

    // The index and a built file open without a token.
    let (status, body) = get(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("scaffoldry-web"));
    let (status, body) = get(&app, "/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "console.log(1)");

    // An unknown page path is a client-side route: it gets the index.
    let (status, body) = get(&app, "/apps/foo").await;
    assert!(body.contains("scaffoldry-web"), "SPA fallback missing: {status}");

    // The API stays guarded, and an unknown API path never gets the index.
    let (status, body) = get(&app, "/api/v1/workspaces").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("scaffoldry-web"));
    let (status, body) = get(&app, "/api/v1/no-such-route").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("scaffoldry-web"));
    let (status, body) = get(&app, "/.well-known/nothing").await;
    assert!(!body.contains("scaffoldry-web"), "{status}");
    let (_, body) = get(&app, "/scim/v2/nothing").await;
    assert!(!body.contains("scaffoldry-web"));

    // A guarded route outside /api still needs a token.
    let (status, body) = get(&app, "/admin/access/grants").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("scaffoldry-web"));

    // A browser navigation to a console page that shares its path with an API route gets the index.
    let html = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
    let token = scaffoldry_server::service::identity::issue_test_token_and_user("jordan.lee@state.edu");
    for path in ["/admin/jobs", "/admin/authority", "/admin/workspaces", "/admin/access/grants"] {
        let (status, body) = send(&app, path, Some(html), None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains("scaffoldry-web"), "{path} did not get the index: {body}");
        // The same path for the web client (a token, JSON) still answers with the API.
        let (status, body) = send(&app, path, Some("application/json"), Some(&token)).await;
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{path}");
        assert!(!body.contains("scaffoldry-web"), "{path} API call got the index");
        // Without a token an API client is still refused.
        let (status, body) = send(&app, path, Some("application/json"), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        assert!(!body.contains("scaffoldry-web"));
        let (status, _) = send(&app, path, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
    // A browser that carries a token still gets the API, and /api paths never get the index.
    let (status, body) = send(&app, "/admin/jobs", Some(html), Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("scaffoldry-web"));
    let (status, body) = send(&app, "/api/v1/admin/jobs", Some(html), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("scaffoldry-web"));

    std::fs::remove_dir_all(&dir).ok();
}

async fn send(app: &axum::Router, uri: &str, accept: Option<&str>, token: Option<&str>) -> (StatusCode, String) {
    let mut b = Request::builder().uri(uri);
    if let Some(a) = accept {
        b = b.header("accept", a);
    }
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let res = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}
