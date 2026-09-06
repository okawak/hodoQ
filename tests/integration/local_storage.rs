use hodoq::domain::ViewKind;
use hodoq::infrastructure::{AppPaths, AppSettings, InstanceLock, RepositoryError};
use std::fs;

#[test]
fn absent_view_preference_defaults_to_list() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    assert_eq!(AppSettings::load(&path).view_kind, ViewKind::List);
    fs::write(&path, r#"{"active_view":"all"}"#).unwrap();
    assert_eq!(AppSettings::load(&path).view_kind, ViewKind::List);
}

#[test]
fn corrupt_settings_fall_back_to_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    fs::write(&path, "not json").unwrap();
    let settings = AppSettings::load(&path);
    assert_eq!(settings.window.width, 1280.0);
    assert_eq!(settings.theme, "dark");
}

#[test]
fn loading_settings_normalizes_out_of_range_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let mut settings = AppSettings::default();
    settings.window.width = 10.0;
    settings.sidebar_width = 10_000.0;
    settings.sort.clear();
    settings.save(&path).unwrap();

    // The unit test covers normalize itself; retain coverage that load calls it.
    let reloaded = AppSettings::load(&path);
    assert_eq!(reloaded.window.width, 1280.0);
    assert_eq!(reloaded.sidebar_width, 380.0);
    assert_eq!(reloaded.sort.len(), 1);
}

#[test]
fn settings_round_trip_uses_the_resolved_data_directory() {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::resolve(Some(directory.path())).unwrap();
    for kind in [ViewKind::List, ViewKind::Board, ViewKind::Calendar] {
        let settings = AppSettings {
            active_view: "all".to_owned(),
            view_kind: kind,
            sidebar_width: 240.0,
            ..AppSettings::default()
        };
        settings.save(&paths.settings).unwrap();
        let reloaded = AppSettings::load(&paths.settings);
        assert_eq!(
            serde_json::to_value(&reloaded).unwrap(),
            serde_json::to_value(&settings).unwrap()
        );
    }
}

#[test]
fn second_instance_with_same_data_directory_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("hodoq.lock");
    let first = InstanceLock::acquire(&path).unwrap();
    assert!(matches!(
        InstanceLock::acquire(&path),
        Err(RepositoryError::AlreadyRunning)
    ));
    drop(first);
    assert!(InstanceLock::acquire(&path).is_ok());
}
