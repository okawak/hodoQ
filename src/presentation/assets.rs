use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

pub(super) struct Assets;

const ICONS: &[(&str, &[u8])] = &[
    (
        "icons/calendar.svg",
        include_bytes!("../../assets/icons/calendar.svg"),
    ),
    (
        "icons/arrow-left.svg",
        include_bytes!("../../assets/icons/arrow-left.svg"),
    ),
    (
        "icons/arrow-right.svg",
        include_bytes!("../../assets/icons/arrow-right.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/chevron-up.svg",
        include_bytes!("../../assets/icons/chevron-up.svg"),
    ),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::{IconName, IconNamed as _};

    #[test]
    fn due_picker_icons_are_embedded() {
        for icon in [
            IconName::Calendar,
            IconName::ArrowLeft,
            IconName::ArrowRight,
            IconName::ChevronDown,
            IconName::ChevronUp,
        ] {
            assert!(Assets.load(icon.path().as_ref()).unwrap().is_some());
        }
        assert_eq!(Assets.list("icons/").unwrap().len(), 5);
        assert!(Assets.load("icons/unknown.svg").unwrap().is_none());
    }
}
