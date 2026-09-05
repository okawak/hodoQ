use hodoq::{domain::Task, infrastructure::SqliteRepository};
use std::fs;
use time::OffsetDateTime;

#[test]
fn csv_export_has_optional_bom_and_consistent_columns() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tasks.csv");
    let mut task = Task::new("comma, title", OffsetDateTime::UNIX_EPOCH).unwrap();
    task.memo = "line 1\nline 2".to_owned();
    SqliteRepository::export_tasks_csv(&path, &[task], true).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    let mut reader = csv::Reader::from_reader(bytes.as_slice());
    assert_eq!(reader.headers().unwrap().len(), 11);
    assert_eq!(reader.records().next().unwrap().unwrap().len(), 11);

    SqliteRepository::export_tasks_csv(&path, &[], false).unwrap();
    assert!(!fs::read(path).unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
}
