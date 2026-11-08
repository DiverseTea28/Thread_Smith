//! Try the generator from the command line, without the window.
//!
//! ```text
//! cargo run --release -p ts-gen --example generate -- check
//! cargo run --release -p ts-gen --example generate -- images shirts "red flannel, rolled sleeves"
//! cargo run --release -p ts-gen --example generate -- images jackets "denim" C:\path\to\reference.png
//! cargo run --release -p ts-gen --example generate -- mesh C:\path\to\picture.png balanced
//! ```
//! Uses ComfyUI at 127.0.0.1:8188 (or the port in THREADSMITH_COMFY_PORT) and
//! starts it if it isn't running.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ts_gen::{EngineSettings, GenEvent, GenJob, GenRequest, Launcher, Presets, Quality};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ai_dir = ts_gen::ai_dir().expect("ai folder not found (run from the ThreadSmith folder)");
    let presets = Presets::load(&ai_dir.join("presets/garment_presets.json")).unwrap();
    let models = ts_gen::models::ModelList::load(&ai_dir.join("models.json")).unwrap().sets;
    let mut settings = EngineSettings::default();
    if let Ok(p) = std::env::var("THREADSMITH_COMFY_PORT") {
        settings.port = p.parse().expect("bad port");
    }
    if std::env::var("THREADSMITH_NO_START").is_ok() {
        settings.auto_start = false;
    }
    let request = match args.first().map(String::as_str) {
        Some("check") => GenRequest::Check,
        Some("images") => {
            let garment = presets.find(args.get(1).map(String::as_str).unwrap_or("shirts")).expect("unknown garment id");
            let text = args.get(2).cloned().unwrap_or_default();
            let reference = args.get(3).map(PathBuf::from);
            let prompt = presets.prompt(garment, &text, reference.is_some());
            println!("Prompt: {prompt}");
            GenRequest::Images { prompt, reference, count: 2, seed: ts_gen::random_seed() }
        }
        Some("mesh") => {
            let image = PathBuf::from(args.get(1).expect("give a picture"));
            let quality = match args.get(2).map(String::as_str) {
                Some("fast") => Quality::Fast,
                Some("best") => Quality::Best,
                _ => Quality::Balanced,
            };
            GenRequest::Mesh { image, quality, faces: 150_000, seed: 1 }
        }
        _ => {
            eprintln!("Use: check | images <garment> <text> [picture] | mesh <picture> [fast|balanced|best]");
            std::process::exit(2);
        }
    };
    let launcher = Arc::new(Mutex::new(Launcher::default()));
    let job = GenJob::start(settings, launcher.clone(), ai_dir, models, request, || {});
    let mut ok = true;
    for ev in job.rx.iter() {
        match ev {
            GenEvent::Status(s) => println!("{s}"),
            GenEvent::Progress { label, fraction } => println!("[{:>3.0}%] {label}", fraction * 100.0),
            GenEvent::Image(p) => println!("Picture: {}", p.display()),
            GenEvent::Mesh(p) => println!("3D model: {}", p.display()),
            GenEvent::Report(r) => println!("{r:?}"),
            GenEvent::Finished(r) => {
                if let Err(e) = r {
                    eprintln!("Failed: {e}");
                    ok = false;
                }
                break;
            }
        }
    }
    // Leave ComfyUI running if we didn't start it; stop it if we did.
    launcher.lock().unwrap().stop();
    std::process::exit(if ok { 0 } else { 1 });
}
