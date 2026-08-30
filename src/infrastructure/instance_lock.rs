use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use super::RepositoryError;

pub struct InstanceLock {
    file: File,
}

impl InstanceLock {
    pub fn acquire(path: &Path) -> Result<Self, RepositoryError> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        file.try_lock()
            .map_err(|_| RepositoryError::AlreadyRunning)?;
        Ok(Self { file })
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
