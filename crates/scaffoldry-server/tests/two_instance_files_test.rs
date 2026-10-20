//! Stateless server phase 6: the writes that need a files directory, proven across two instances.
//!
//! Uploads, imports and exports store files under `SCAFFOLDRY_FILES_DIR`, a setting of the process. A test may not set
//! it for the others, so these cases run here. This binary holds one test, so the setting reaches nothing else.

mod common;
mod two_instance;

use two_instance::Entry;

#[tokio::test]
async fn writes_that_need_a_files_directory_are_read_through_the_other_instance() {
    let dir = std::env::temp_dir().join(format!("scaffoldry-p6-files-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("a files directory");
    std::env::set_var("SCAFFOLDRY_FILES_DIR", &dir);
    // No job runner drains the queue here, so a person's queued jobs would meet the limit. The limits have their own tests.
    std::env::set_var("SCAFFOLDRY_JOB_LIMIT_PER_USER", "100000");
    std::env::set_var("SCAFFOLDRY_JOB_LIMIT_QUEUED", "100000");

    let world = two_instance::world().await;
    let (ran, failures) = two_instance::run_cases(&world, |e| if let Entry::FilesCase(c) = e { Some(c) } else { None }).await;
    println!("{ran} cases ran");
    assert!(ran > 0, "there are cases that need a files directory");
    assert!(failures.is_empty(), "{} of {ran} cases failed:\n{}", failures.len(), failures.join("\n"));
    let _ = std::fs::remove_dir_all(&dir);
}
