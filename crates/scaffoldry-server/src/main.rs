//! Scaffoldry API Server Daemon Entrypoint

use scaffoldry_server::{build_app_with_state, jobs, service, state::ServerState};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "setup-token" {
        let state = ServerState::new()?;
        if let Some(ref repo) = state.repository {
            repo.revoke_setup_tokens()?;
        }
        let token = service::identity::create_setup_token(&state)?;
        println!("{token}");
        return Ok(());
    }

    if args.len() > 1 && args[1] == "seed-demo" {
        let state = ServerState::new()?;
        state.seed_demo()?;
        println!("Demo data seeded successfully.");
        return Ok(());
    }

    let state = Arc::new(ServerState::new()?);
    if service::identity::boot_setup_token_needed(&state)
        && service::identity::create_setup_token(&state).is_ok()
    {
        println!("Boot setup token created.");
    }

    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()
        .unwrap_or(8080);

    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;

    // Workers start after migrations. With no job kinds registered there is nothing to claim, so
    // none start. The runner stops its workers when it is dropped.
    let _job_runner = match state.repository.as_deref() {
        Some(repo) if !jobs::JOB_KINDS.is_empty() => {
            Some(jobs::JobRunner::start(repo, jobs::JOB_KINDS, jobs::JobConfig::default())?)
        }
        _ => None,
    };

    let app = build_app_with_state(state)?;

    println!("Scaffoldry Sovereign API Server listening on http://{}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
