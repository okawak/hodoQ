//! Daily backup scheduling and retention, independent of the GUI.
use super::{ApplicationError, TaskApplication};
use crate::infrastructure::AppPaths;
use std::{fs, path::PathBuf};
use time::{OffsetDateTime, UtcOffset};

pub(crate) fn schedule_daily_backup(
    worker: &TaskApplication,
    paths: &AppPaths,
) -> Result<(), ApplicationError> {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let today = OffsetDateTime::now_utc().to_offset(offset).date();
    let destination = paths.backups.join(format!("hodoq-{today}.sqlite3"));
    if destination.exists() {
        return Ok(());
    }
    worker.create_backup(destination)?;
    let mut backups = fs::read_dir(&paths.backups)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_automatic_backup(path))
        .collect::<Vec<PathBuf>>();
    backups.sort();
    while backups.len() > 5 {
        if let Some(oldest) = backups.first().cloned() {
            fs::remove_file(oldest)?;
            backups.remove(0);
        }
    }
    Ok(())
}

pub(crate) fn schedule_maintenance(worker: TaskApplication, paths: AppPaths) {
    let _ = std::thread::Builder::new()
        .name("hodoq-maintenance".to_owned())
        .spawn(move || {
            let _ = schedule_daily_backup(&worker, &paths);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(24 * 60 * 60));
                let _ = schedule_daily_backup(&worker, &paths);
            }
        });
}

fn is_automatic_backup(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.len() != "hodoq-YYYY-MM-DD.sqlite3".len()
        || !name.starts_with("hodoq-")
        || !name.ends_with(".sqlite3")
    {
        return false;
    }
    let date = &name[6..16];
    date.bytes().enumerate().all(|(index, byte)| match index {
        4 | 7 => byte == b'-',
        _ => byte.is_ascii_digit(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_retention_does_not_match_manual_backups() {
        assert!(is_automatic_backup(std::path::Path::new(
            "hodoq-2026-08-28.sqlite3"
        )));
        assert!(!is_automatic_backup(std::path::Path::new(
            "hodoq-manual-123.sqlite3"
        )));
        assert!(!is_automatic_backup(std::path::Path::new(
            "hodoq-before-restore-123.sqlite3"
        )));
    }
}
