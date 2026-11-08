// Hide the black console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod animation;
mod app;
mod bridge;
mod camera;
mod garment;
mod generate;
mod project;
mod tools;
mod ui_icons;
mod viewport;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"),
    )
    .init();

    // Optional: `threadsmith body.fbx [garment.fbx]` opens files at start.
    // `-` as the body means "no body yet" (wait for one from Unreal).
    let open_path = std::env::args()
        .nth(1)
        .filter(|a| a != "-")
        .map(std::path::PathBuf::from);
    let garment_path = std::env::args().nth(2).map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ThreadSmith")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_256.png"))
                    .unwrap_or_default(),
            ),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "ThreadSmith",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::ThreadSmithApp::new(
                cc,
                open_path,
                garment_path,
            )))
        }),
    )
}
