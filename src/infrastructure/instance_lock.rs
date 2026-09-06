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
