//! Serving, and stopping cleanly (stateless server phase 6).

use axum::Router;
use std::future::Future;
use std::time::Duration;
use tokio::net::TcpListener;

/// Serves `app` until `shutdown` resolves. Then it takes no new connections and waits for the requests it holds, up
/// to `grace`. A request still running when the grace period ends is dropped, so one stuck request cannot hold the stop.
pub async fn serve_until(
    listener: TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
    grace: Duration,
) -> std::io::Result<()> {
    let (stopping, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = std::future::IntoFuture::into_future(axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown.await;
        let _ = stopping.send(());
    }));
    tokio::pin!(server);
    tokio::select! {
        done = &mut server => done,
        _ = async { let _ = stopped.await; tokio::time::sleep(grace).await } => Ok(()),
    }
}

/// Resolves on `SIGTERM` or an interrupt (Ctrl-C).
pub async fn termination() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("a SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}
