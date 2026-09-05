use hodoq::{
    domain::{SortSpec, Task, TaskFilter},
    infrastructure::SqliteRepository,
};
use std::time::Duration;
use time::OffsetDateTime;

#[test]
fn ten_thousand_tasks_can_be_saved_and_loaded() {
    check_ten_thousand_task_round_trip();
}

#[test]
#[ignore = "run performance_ tests in release mode with --test-threads=1"]
#[allow(clippy::assertions_on_constants)]
fn performance_ten_thousand_task_round_trip() {
    // An explicit --ignored debug run should fail, not report misleading timings.
    assert!(
        !cfg!(debug_assertions),
        "performance tests require --release"
    );
    let (round_trip, load, search) = check_ten_thousand_task_round_trip();
    eprintln!("10,000 tasks: round trip={round_trip:?}, load={load:?}, search={search:?}");
    assert!(
        round_trip < Duration::from_secs(5),
        "10,000 task round trip took {round_trip:?}"
    );
    assert!(
        load < Duration::from_secs(1),
        "10,000 task load took {load:?}"
    );
    assert!(
        search < Duration::from_millis(100),
        "10,000 task search took {search:?}"
    );
}

// Keep the same dataset and correctness checks in functional and performance runs.
fn check_ten_thousand_task_round_trip() -> (Duration, Duration, Duration) {
    let mut repository = SqliteRepository::open_in_memory().unwrap();
    let now = OffsetDateTime::UNIX_EPOCH;
    let tasks = (0..10_000)
        .map(|index| Task::new(format!("task {index}"), now).unwrap())
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    repository.save_tasks(&tasks).unwrap();
    let load_started = std::time::Instant::now();
    let loaded = repository.list_all_tasks().unwrap();
    let load_elapsed = load_started.elapsed();
    let round_trip_elapsed = started.elapsed();

    assert_eq!(loaded.len(), 10_000);
    let search_started = std::time::Instant::now();
    let filter = TaskFilter {
        query: "task 9999".to_owned(),
        ..TaskFilter::default()
    };
    let matches = repository
        .list_tasks(&filter, &[SortSpec::default()])
        .unwrap();
    let search_elapsed = search_started.elapsed();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, tasks[9999].id);
    (round_trip_elapsed, load_elapsed, search_elapsed)
}
