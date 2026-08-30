use gpui::Rgba;

const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

pub const BACKGROUND: Rgba = rgb(0x0d, 0x11, 0x17);
pub const SURFACE: Rgba = rgb(0x16, 0x1b, 0x22);
pub const SURFACE_HOVER: Rgba = rgb(0x1f, 0x26, 0x30);
pub const BORDER: Rgba = rgb(0x30, 0x36, 0x3d);
pub const TEXT: Rgba = rgb(0xe6, 0xed, 0xf3);
pub const MUTED: Rgba = rgb(0x8b, 0x94, 0x9e);
pub const ACCENT: Rgba = rgb(0x2f, 0x81, 0xf7);
pub const SUCCESS: Rgba = rgb(0x3f, 0xb9, 0x50);
pub const WARNING: Rgba = rgb(0xd2, 0x99, 0x22);
pub const DANGER: Rgba = rgb(0xf8, 0x51, 0x49);
