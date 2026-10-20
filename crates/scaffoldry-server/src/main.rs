//! Scaffoldry API Server Daemon Entrypoint

use scaffoldry_server::{build_app_with_state, files, jobs, serve, service, state::ServerState};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "setup-token" {
        let state = ServerState::new()?;
        {
        let repo = &state.repository;
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

    // A files directory that is missing or read-only stops the server here, not on the first upload.
    match files::open_dir() {
        Ok(Some(dir)) => println!("Files are kept in {}.", dir.display()),
        Ok(None) => println!("SCAFFOLDRY_FILES_DIR is not set. Uploads are off."),
        Err(e) => return Err(e.into()),
    }

    let port: u16 = env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()
        .unwrap_or(8080);

    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;

    // Workers start after migrations. With no job kinds registered there is nothing to claim, so
    // none start. The runner stops its workers when it is dropped.
    let repo = &*state.repository;
    // Finished jobs are cleared once a day.
    jobs::register_schedule(repo, "purge_jobs", 24 * 60 * 60)?;
    // Uploads that no record took are cleared every hour.
    jobs::register_schedule(repo, "purge_unattached_files", 60 * 60)?;
    // A safety net for uploads whose scan job could not be queued.
    jobs::register_schedule(repo, "scan_file", 10 * 60)?;
    let job_runner = jobs::JobRunner::start(repo, jobs::JOB_KINDS, jobs::JobConfig::default())?;

    let app = build_app_with_state(state)?;

    println!("Scaffoldry Sovereign API Server listening on http://{}", addr);
    let listener = TcpListener::bind(addr).await?;
    // On SIGTERM the server takes no new connections, finishes the requests it holds for up to 30 seconds, and returns.
    serve::serve_until(listener, app, serve::termination(), Duration::from_secs(30)).await?;
    println!("Stopped taking requests. Waiting for the job in progress.");
    // Dropping the runner stops its workers after the job each is on, so a job is never cut off halfway.
    drop(job_runner);

    Ok(())
}
