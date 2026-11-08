# AI generation (how it works)

ThreadSmith makes garments with free local AI. It doesn't run the models
itself: it drives a **local ComfyUI** in the background through ComfyUI's HTTP
API (only on this PC, `127.0.0.1`). This is the same idea as the Krita AI
plugin. ComfyUI handles GPU memory and model loading, and new models are
just new workflow files.

## The steps

1. **Pick a type** (Shirts, Jackets, Pants…). Each type has a hidden preset in
   `ai/presets/garment_presets.json`.
2. **Describe it**: text, a reference picture, or both. ThreadSmith wraps your
   words in the preset's prompt: a "ghost mannequin" product photo (clothing
   shown hollow, with no mannequin) with the openings asked for
   ("open sleeve cuffs, open neck…").
3. **Make pictures**: FLUX.2 [klein] 4B draws 1 to 4 versions
   (`ai/workflows/image_from_text.json`, or `image_from_image.json` with a picture).
4. **Make 3D**: TRELLIS.2 turns the picked picture into a textured GLB
   (`ai/workflows/garment_3d.json`, or `garment_3d_fast.json` for Fast). Hole
   filling is turned off so openings stay open.
5. **Placed on the body**: the preset's `place` rule turns it to face the body,
   sizes it from bone spacing (e.g. the clavicles for tops), lines up its top
   or bottom edge with a bone, and splits pairs (shoes, gloves) onto each side.
   Hollow cloth is then grown just enough to clear the body. Rigid types
   (Head, Belts, Backpacks, Accessory) switch Auto skin to Rigid.

Then use the normal steps: check placement, Fit to body, Auto skin.

## Files

| Path | What it is |
| --- | --- |
| `ai/presets/garment_presets.json` | Garment types: hidden prompt text, placement, skinning. Edit or add types here. |
| `ai/presets/icons/*.png` | Type icons (Tabler icons, MIT, plus a few drawn for ThreadSmith). |
| `ai/workflows/*.json` | ComfyUI workflows in API format. `"$NAME"` blanks are filled in by ThreadSmith. Keys starting with `_` are notes and are removed before sending. |
| `ai/models.json` | Model files and download links (put into ComfyUI's `models` folder). |
| `crates/ts-gen` | The Rust side: presets, ComfyUI client, model downloader, background jobs. |
| `crates/ts-app/src/generate.rs` | The Generate window. |

Results are saved in `Documents\ThreadSmith\Generated\run_…\`. ComfyUI's own
log, when ThreadSmith starts it, is `Documents\ThreadSmith\Generated\comfyui_log.txt`.

## What it needs (for now)

- **ComfyUI** (ComfyUI-Easy-Install works) with Torch 2.8.0+cu128.
- The **ComfyUI-Trellis2** nodes (Easy-Install: `Add-Ons\Trellis2 (requires Torch 2.8.0+cu128).bat`).
- The models in `ai/models.json`. The Generate window's Settings can download them.

ThreadSmith finds ComfyUI-Easy-Install in `C:\ComfyUI-Easy-Install` on its own, or
use Settings → Browse… It starts ComfyUI hidden if it isn't running, and
closes it again when ThreadSmith closes.

**Planned:** a one-click "Set up AI" that installs a private ComfyUI + TRELLIS.2
into `Documents\ThreadSmith\AI\Engine`, so new users don't need ComfyUI
already. ThreadSmith already looks there.

## Testing without the window

```
cargo run --release -p ts-gen --example generate -- check
cargo run --release -p ts-gen --example generate -- images jackets "worn brown leather"
cargo run --release -p ts-gen --example generate -- mesh "C:\path\to\picture.png" balanced
```

## Licenses

FLUX.2 [klein] 4B: Apache 2.0. TRELLIS.2: MIT. ComfyUI-Trellis2 nodes: MIT.
ComfyUI is GPL-3.0; ThreadSmith only talks to it over HTTP as a separate program.
