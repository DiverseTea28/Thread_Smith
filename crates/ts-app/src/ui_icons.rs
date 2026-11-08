//! ThreadSmith's own interface icons (white, tinted when drawn).

use egui::{vec2, TextureHandle};

pub struct UiIcons {
    /// Cloud with an up arrow: sending things out (to Unreal).
    pub export: TextureHandle,
    /// Cloud with a down arrow: bringing things in (opening files).
    pub import: TextureHandle,
    /// Wardrobe doors: the outfit library.
    pub wardrobe: TextureHandle,
    /// The "threadsmith" logo with its name, for the empty viewport.
    pub logo_text: TextureHandle,
    /// Sparkles: AI generation (Tabler icon, MIT).
    pub generate: TextureHandle,
    /// Needle and thread: fitting (Tabler icon, MIT).
    pub fitment: TextureHandle,
    /// Show / hide / delete buttons in the Layers list (Tabler icons, MIT).
    pub eye: TextureHandle,
    pub eye_off: TextureHandle,
    pub trash: TextureHandle,
}

impl UiIcons {
    pub fn load(ctx: &egui::Context) -> Self {
        let load = |name: &str, bytes: &[u8]| {
            let img = image::load_from_memory(bytes).expect("built-in icon").to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            ctx.load_texture(
                format!("ui-{name}"),
                egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
                egui::TextureOptions::LINEAR,
            )
        };
        Self {
            export: load("export", include_bytes!("../assets/ui/export.png")),
            import: load("import", include_bytes!("../assets/ui/import.png")),
            wardrobe: load("wardrobe", include_bytes!("../assets/ui/wardrobe.png")),
            logo_text: load("logo_text", include_bytes!("../assets/ui/logo_text.png")),
            generate: load("generate", include_bytes!("../assets/ui/generate.png")),
            fitment: load("fitment", include_bytes!("../assets/ui/fitment.png")),
            eye: load("eye", include_bytes!("../assets/ui/eye.png")),
            eye_off: load("eye_off", include_bytes!("../assets/ui/eye_off.png")),
            trash: load("trash", include_bytes!("../assets/ui/trash.png")),
        }
    }
}

/// A small icon (16 px) for buttons and headers.
pub fn small(icon: &TextureHandle) -> egui::Image<'static> {
    egui::Image::new(egui::load::SizedTexture::new(icon.id(), vec2(16.0, 16.0)))
        .tint(egui::Color32::from_gray(225))
}

/// A button with an icon before its text.
pub fn button<'a>(icon: &TextureHandle, text: impl Into<egui::WidgetText>) -> egui::Button<'a> {
    egui::Button::image_and_text(small(icon), text)
}
