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

    // Re-encrypts every stored secret from one master key file to another, in one transaction.
    if args.len() > 1 && args[1] == "rotate-master-key" {
        if args.len() != 4 {
            return Err("Usage: scaffoldry-server rotate-master-key {old_key_file} {new_key_file}".into());
        }
        let state = ServerState::new()?;
        let count = scaffoldry_server::secrets::rotate_master_key(&state.repository, std::path::Path::new(&args[2]), std::path::Path::new(&args[3]))?;
        println!("Re-encrypted {count} secrets. Point SCAFFOLDRY_MASTER_KEY_FILE at the new key file and restart.");
        return Ok(());
    }

    if args.len() > 1 && args[1] == "seed-demo" {
        let state = ServerState::new()?;
        state.seed_demo()?;
        println!("Demo data seeded successfully.");
        return Ok(());
    }

    if args.len() > 1 && args[1] == "synthesize" {
        let mut opts = service::synthesize::SynthesizeOptions::default();
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--seed" if i + 1 < args.len() => {
                    opts.seed = args[i + 1].parse()?;
                    i += 2;
                }
                "--people" if i + 1 < args.len() => {
                    opts.people = args[i + 1].parse()?;
                    i += 2;
                }
                "--units" if i + 1 < args.len() => {
                    opts.units = args[i + 1].parse()?;
                    i += 2;
                }
                "--hierarchies" if i + 1 < args.len() => {
                    opts.hierarchies = args[i + 1].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                    i += 2;
                }
                "--apps" if i + 1 < args.len() => {
                    opts.apps = args[i + 1].parse()?;
                    i += 2;
                }
                "--records" if i + 1 < args.len() => {
                    opts.records = args[i + 1].parse()?;
                    i += 2;
                }
                _ => {
                    i += 1;
                }
            }
        }
        let state = Arc::new(ServerState::new()?);
        let hash = service::synthesize::synthesize(&state, &opts)?;
        println!("Synthetic institution created with seed {}. World hash: {}", opts.seed, hash);
        return Ok(());
    }

    let state = Arc::new(ServerState::new()?);
    // Stored secrets need their master key. Without it, or with the wrong one, the server does not start.
    scaffoldry_server::secrets::boot_check(&state.repository, state.master_key_file.as_deref())?;
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
    // Old notices are cleared once a day.
    jobs::register_schedule(repo, "purge_notifications", 24 * 60 * 60)?;
    // Mail that is due is sent every 15 seconds. Nothing is sent while mail.enabled is false.
    jobs::register_schedule(repo, "send_email", 15)?;
    // Each person's digest goes out at 07:00 in the institution's zone.
    scaffoldry_server::mail::register_digest_schedule(repo)?;
    // The policy library's web sources are checked every hour. Each one is fetched at most once a day.
    jobs::register_schedule(repo, "library_fetch", 60 * 60)?;
    // Access receipts are sealed into a hash-chained batch every minute.
    jobs::register_schedule(repo, "seal_receipts", 60)?;
    // Retention destroys what its policies no longer keep, nightly at 02:00 in the institution's zone.
    let zone = repo.get_platform_settings()?.get("platform.timezone").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "UTC".to_string());
    jobs::register_daily_schedule(repo, "purge_expired", &zone, "02:00:00")?;
    // Files no record holds any more are removed, once their 24 hours of grace are over.
    jobs::register_daily_schedule(repo, "purge_orphan_blobs", &zone, "03:00:00")?;
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
