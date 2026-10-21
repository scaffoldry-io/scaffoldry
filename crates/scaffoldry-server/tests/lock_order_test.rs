//! A request that holds a read lock must not ask for the same lock again while a writer waits. The standard library's
//! lock lets a waiting writer block every new reader, so a nested read plus a refresh that writes is a deadlock.
//! `member_role` reads workspaces, collaborators, the unit tree and organizations, so no caller may hold those across it.

mod common;

use common::TestDb;
use scaffoldry_server::state::AuthUser;
use std::sync::mpsc;
use std::time::Duration;

fn caller() -> AuthUser {
    AuthUser {
        eppn: "einstein@physics.state.edu".to_string(),
        name: "Albert Einstein".to_string(),
        role_title: "Professor".to_string(),
        affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        view_as_by: None,
    }
}

/// The cost of a role lookup, printed and not asserted: a list of the demo workspaces, one caller, nothing contending.
#[test]
fn a_workspace_list_costs_a_few_database_reads_per_workspace() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let me = caller();
    let count = scaffoldry_server::service::workspaces::list_workspaces(&me, &state).expect("a list").len();
    let started = std::time::Instant::now();
    for _ in 0..50 {
        scaffoldry_server::service::workspaces::list_workspaces(&me, &state).expect("a list");
    }
    eprintln!("list_workspaces: {:.1} ms for {} visible workspaces", started.elapsed().as_secs_f64() * 1000.0 / 50.0, count);
}

/// Runs readers that list workspaces while a writer takes each lock they use, and fails if they do not all finish.
#[test]
fn listing_workspaces_does_not_deadlock_against_a_writer() {
    let db = TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");

    let (done_tx, done_rx) = mpsc::channel::<&'static str>();
    let mut handles = Vec::new();
    for _ in 0..6 {
        let (state, tx) = (state.clone(), done_tx.clone());
        handles.push(std::thread::spawn(move || {
            let me = caller();
            for _ in 0..150 {
                let _ = scaffoldry_server::service::workspaces::list_workspaces(&me, &state);
            }
            let _ = tx.send("reader");
        }));
    }
    {
        let (state, tx) = (state.clone(), done_tx.clone());
        handles.push(std::thread::spawn(move || {
            for _ in 0..600 {
                drop(state.workspaces.write().unwrap_or_else(|p| p.into_inner()));
                drop(state.collaborators.write().unwrap_or_else(|p| p.into_inner()));
                drop(state.tree.write().unwrap_or_else(|p| p.into_inner()));
                drop(state.organizations.write().unwrap_or_else(|p| p.into_inner()));
            }
            let _ = tx.send("writer");
        }));
    }
    drop(done_tx);
    for _ in 0..7 {
        done_rx.recv_timeout(Duration::from_secs(60)).expect("a reader or the writer is stuck: the locks deadlock");
    }
    for h in handles {
        h.join().unwrap();
    }
}
