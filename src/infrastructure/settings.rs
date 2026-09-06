use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::domain::{GroupBy, SortSpec, ViewKind};

use super::RepositoryError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub settings_version: u32,
    pub window: WindowSettings,
    pub sidebar_width: f32,
    pub detail_width: f32,
    pub active_view: String,
    pub view_kind: ViewKind,
    pub sort: Vec<SortSpec>,
    pub group_by: Option<GroupBy>,
    pub theme: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            settings_version: 1,
            window: WindowSettings::default(),
            sidebar_width: 280.0,
            detail_width: 360.0,
            active_view: "today".to_owned(),
            view_kind: ViewKind::List,
            sort: vec![SortSpec::default()],
            group_by: None,
            theme: "dark".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: 1280.0,
            height: 800.0,
            maximized: false,
        }
    }
}

impl AppSettings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default();
        settings.normalize();
        settings
    }

    pub fn save(&self, path: &Path) -> Result<(), RepositoryError> {
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        if let Err(error) = fs::rename(&temporary, path) {
            if path.exists() {
                fs::remove_file(path)?;
                fs::rename(temporary, path)?;
            } else {
                return Err(error.into());
            }
        }
        Ok(())
    }

    fn normalize(&mut self) {
        let defaults = Self::default();
        if !self.window.width.is_finite() || !(900.0..=7680.0).contains(&self.window.width) {
            self.window.width = defaults.window.width;
        }
        if !self.window.height.is_finite() || !(600.0..=4320.0).contains(&self.window.height) {
            self.window.height = defaults.window.height;
        }
        if self.window.x.is_some_and(|value| !value.is_finite()) {
            self.window.x = None;
        }
        if self.window.y.is_some_and(|value| !value.is_finite()) {
            self.window.y = None;
        }
        if !self.sidebar_width.is_finite() {
            self.sidebar_width = defaults.sidebar_width;
        }
        self.sidebar_width = self.sidebar_width.clamp(180.0, 380.0);
        if !self.detail_width.is_finite() {
            self.detail_width = defaults.detail_width;
        }
        self.detail_width = self.detail_width.clamp(280.0, 560.0);
        if self.sort.is_empty() {
            self.sort.push(SortSpec::default());
        }
        self.sort.truncate(2);
        self.theme = "dark".to_owned();
        self.settings_version = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_range_settings_are_normalized() {
        let mut settings = AppSettings::default();
        settings.window.width = 10.0;
        settings.sidebar_width = 10_000.0;
        settings.sort.clear();
        settings.normalize();
        assert_eq!(settings.window.width, 1280.0);
        assert_eq!(settings.sidebar_width, 380.0);
        assert_eq!(settings.sort.len(), 1);
    }
}
