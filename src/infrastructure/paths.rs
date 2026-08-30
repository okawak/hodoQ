use std::{
    env, fs,
    path::{Path, PathBuf},
};

use super::{InstanceLock, RepositoryError};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub settings: PathBuf,
    pub backups: PathBuf,
    pub exports: PathBuf,
    pub logs: PathBuf,
    pub lock: PathBuf,
}

impl AppPaths {
    pub fn resolve(override_dir: Option<&Path>) -> Result<Self, RepositoryError> {
        let uses_repository_default = override_dir.is_none();
        let data_dir = match override_dir {
            Some(path) => path.to_path_buf(),
            None => repository_root()?.join(".hodoq"),
        };
        let paths = Self {
            database: data_dir.join("tasks.sqlite3"),
            settings: data_dir.join("settings.json"),
            backups: data_dir.join("backups"),
            exports: data_dir.join("exports"),
            logs: data_dir.join("logs"),
            lock: data_dir.join("hodoq.lock"),
            data_dir,
        };
        paths.create_directories()?;
        if uses_repository_default {
            paths.import_legacy_data()?;
        }
        Ok(paths)
    }

    fn create_directories(&self) -> Result<(), RepositoryError> {
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(&self.backups)?;
        fs::create_dir_all(&self.exports)?;
        fs::create_dir_all(&self.logs)?;
        Ok(())
    }

    fn import_legacy_data(&self) -> Result<(), RepositoryError> {
        if self.database.exists() {
            return Ok(());
        }
        let Some(legacy_directory) = legacy_data_directory() else {
            return Ok(());
        };
        self.import_legacy_data_from(&legacy_directory)
    }

    fn import_legacy_data_from(&self, legacy_directory: &Path) -> Result<(), RepositoryError> {
        if self.database.exists() {
            return Ok(());
        }
        let legacy_database = legacy_directory.join("tasks.sqlite3");
        if !legacy_database.is_file() || legacy_directory == self.data_dir {
            return Ok(());
        }

        let _legacy_lock = InstanceLock::acquire(&legacy_directory.join("hodoq.lock"))?;
        let importing_database = self.data_dir.join("tasks.sqlite3.importing");
        fs::copy(&legacy_database, &importing_database)?;
        fs::rename(importing_database, &self.database)?;
        copy_if_present(
            &legacy_directory.join("tasks.sqlite3-journal"),
            &self.data_dir.join("tasks.sqlite3-journal"),
        )?;
        copy_if_present(
            &legacy_directory.join("tasks.sqlite3-wal"),
            &self.data_dir.join("tasks.sqlite3-wal"),
        )?;
        copy_if_present(
            &legacy_directory.join("tasks.sqlite3-shm"),
            &self.data_dir.join("tasks.sqlite3-shm"),
        )?;
        copy_if_present(&legacy_directory.join("settings.json"), &self.settings)?;
        copy_directory_contents(&legacy_directory.join("backups"), &self.backups)?;
        copy_directory_contents(&legacy_directory.join("exports"), &self.exports)?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn legacy_data_directory() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("HodoQ"))
}

#[cfg(target_os = "macos")]
fn legacy_data_directory() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).map(|path| {
        path.join("Library")
            .join("Application Support")
            .join("HodoQ")
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn legacy_data_directory() -> Option<PathBuf> {
    None
}

fn copy_if_present(source: &Path, destination: &Path) -> Result<(), RepositoryError> {
    if source.is_file() && !destination.exists() {
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<(), RepositoryError> {
    if !source.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_directory_contents(&source_path, &destination_path)?;
        } else {
            copy_if_present(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn repository_root() -> Result<PathBuf, RepositoryError> {
    let executable_start = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let working_directory_start = env::current_dir().ok();

    executable_start
        .iter()
        .chain(working_directory_start.iter())
        .find_map(|start| repository_root_from(start))
        .ok_or(RepositoryError::RepositoryRootUnavailable)
}

fn repository_root_from(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|directory| {
            directory.join("Cargo.toml").is_file()
                && directory.join("rust-toolchain.toml").is_file()
        })
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_root_is_found_from_release_binary_directory() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("Cargo.toml"),
            "[package]\nname='hodoq'\n",
        )
        .unwrap();
        fs::write(
            temporary.path().join("rust-toolchain.toml"),
            "[toolchain]\n",
        )
        .unwrap();
        let release = temporary.path().join("target").join("release");
        fs::create_dir_all(&release).unwrap();

        assert_eq!(
            repository_root_from(&release),
            Some(temporary.path().to_path_buf())
        );
    }

    #[test]
    fn legacy_database_and_supporting_files_are_copied_once() {
        let temporary = tempfile::tempdir().unwrap();
        let legacy = temporary.path().join("legacy");
        let data_dir = temporary.path().join("repository").join(".hodoq");
        fs::create_dir_all(legacy.join("backups")).unwrap();
        fs::write(legacy.join("tasks.sqlite3"), b"legacy database").unwrap();
        fs::write(legacy.join("settings.json"), b"{}").unwrap();
        fs::write(legacy.join("backups").join("daily.sqlite3"), b"backup").unwrap();

        let paths = AppPaths {
            database: data_dir.join("tasks.sqlite3"),
            settings: data_dir.join("settings.json"),
            backups: data_dir.join("backups"),
            exports: data_dir.join("exports"),
            logs: data_dir.join("logs"),
            lock: data_dir.join("hodoq.lock"),
            data_dir,
        };
        paths.create_directories().unwrap();
        paths.import_legacy_data_from(&legacy).unwrap();

        assert_eq!(fs::read(&paths.database).unwrap(), b"legacy database");
        assert!(paths.settings.is_file());
        assert!(paths.backups.join("daily.sqlite3").is_file());

        fs::write(&paths.database, b"new database").unwrap();
        paths.import_legacy_data_from(&legacy).unwrap();
        assert_eq!(fs::read(&paths.database).unwrap(), b"new database");
    }
}
