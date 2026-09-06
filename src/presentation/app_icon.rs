//! Give direct executable launches and `cargo run` the same Dock icon as the app bundle.
use objc2::{AnyThread as _, MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

const ICON: &[u8] = include_bytes!("../../assets/app-icon/hodoq.png");

fn decode_icon(bytes: &[u8]) -> Option<Retained<NSImage>> {
    NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))
}

pub(super) fn install() {
    let Some(main_thread) = MainThreadMarker::new() else {
        tracing::warn!("application icon must be installed on the main thread");
        return;
    };
    let Some(icon) = decode_icon(ICON) else {
        tracing::warn!("failed to decode the embedded application icon");
        return;
    };
    let application = NSApplication::sharedApplication(main_thread);
    // SAFETY: Called on the main thread with a valid, non-null image retained by AppKit.
    unsafe { application.setApplicationIconImage(Some(&icon)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icon_is_readable_by_appkit() {
        let icon = decode_icon(ICON).expect("application icon must be a valid image");
        assert!(icon.isValid());
        let size = icon.size();
        assert_eq!(size.width, 1024.0);
        assert_eq!(size.height, 1024.0);
    }

    #[test]
    fn bundled_icon_is_readable_by_appkit() {
        let icon = decode_icon(include_bytes!("../../assets/app-icon/HodoQ.icns"))
            .expect("bundle icon must be a valid ICNS image");
        assert!(icon.isValid());
        let size = icon.size();
        assert!(size.width > 0.0);
        assert_eq!(size.width, size.height);
    }
}
