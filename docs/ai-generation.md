# AI generation (how it works)

ThreadSmith makes garments with free local AI, in two parts:

- **Pictures** come from a **local ComfyUI** that ThreadSmith drives in the
  background through ComfyUI's HTTP API (only on this PC, `127.0.0.1`), like
  the Krita AI plugin.
- **3D models** come from **Pixal3D**, run by the **ThreadSmith Engine**
  (`ai/engine/threadsmith_engine.py`), a small Python helper ThreadSmith
  starts hidden with ComfyUI's Python. It sends **live previews** while the
  model forms, and ThreadSmith draws them next to the body: first a cloud of
  cubes (the rough shape), then a grey mesh, then the finished model.
  TRELLIS.2 through a ComfyUI workflow is still there as the other choice
  (Generate > Settings > 3D model).

## The steps

1. **Pick a type** (Shirts, Jackets, Pants…). Each type has a hidden preset in
   `ai/presets/garment_presets.json`.
2. **Describe it**: text, a reference picture, or both. ThreadSmith wraps your
   words in the preset's prompt: a "ghost mannequin" product photo (clothing
   shown hollow, with no mannequin) with the openings asked for
   ("open sleeve cuffs, open neck…").
3. **Make pictures**: FLUX.2 [klein] 4B draws 1 to 4 versions
   (`ai/workflows/image_from_text.json`, or `image_from_image.json` with a picture).
4. **Make 3D**: Pixal3D turns the picked picture into a textured GLB. The
   engine first removes the plain background (flooding in from the edges, so
   light parts *inside* the garment stay), then runs Pixal3D's four stages
   (rough shape, shape, fine detail, colours) and bakes the texture. Only small
   holes are filled, so sleeve and neck openings stay open.
   With TRELLIS.2 chosen instead: `ai/workflows/garment_3d.json` (or
   `garment_3d_fast.json` for Fast) runs in ComfyUI.
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
| `ai/engine/threadsmith_engine.py` | The ThreadSmith Engine (Pixal3D with live previews). Needs only the standard library to start. |
| `ai/setup/Install Pixal3D.bat` | One-time Pixal3D setup (calls `install_pixal3d.py`). Also on the Install Pixal3D… button. |
| `crates/ts-gen` | The Rust side: presets, ComfyUI client, engine client (`engine.rs`), model downloader, background jobs. |
| `crates/ts-app/src/generate.rs` | The Generate tab. |
| `crates/ts-app/src/live_view.rs` | Turns engine previews into something the viewport draws next to the body. |

Results are saved in `Documents\ThreadSmith\Generated\run_…\`. ComfyUI's own
log, when ThreadSmith starts it, is `Documents\ThreadSmith\Generated\comfyui_log.txt`.
The engine's log is `engine_log.txt` in the same folder (Settings > Engine log).

## The ThreadSmith Engine

- Started on the first Make 3D (or Check engine) and kept running, so Pixal3D
  only loads once. It quits by itself when ThreadSmith closes.
- ThreadSmith opens a port on `127.0.0.1` and the engine connects back. Each
  message is `u32 header length | JSON header | u32 data length | data`. The
  message list is at the top of the Python file.
- Before Pixal3D runs, ThreadSmith asks ComfyUI to unload FLUX, so the
  graphics card has room.
- On cards under 22 GB (like a 16 GB 4070 Ti Super) Pixal3D runs in
  low-VRAM mode: one part of the model on the graphics card at a time.
- Quality: Fast = 1024 detail, 8 steps, 1K texture. Balanced = 1024, 12 steps,
  2K. Best = 1536, 12 steps, 2K.
- **Lens** (Settings): how wide a lens Pixal3D assumes the picture was taken
  with. Product pictures are about 20°. If a model comes out stretched or
  squashed front to back, try another value.
- **Test mode:** set `THREADSMITH_ENGINE_FAKE=1` and the engine pretends (a
  growing ball of cubes, then a ball) without a graphics card.
  `THREADSMITH_ENGINE_PYTHON` picks a different Python.

## What it needs (for now)

- **ComfyUI** (ComfyUI-Easy-Install works) with Torch 2.8.0+cu128 and FlashAttention.
- **For Pixal3D:** run `ai\setup\Install Pixal3D.bat` once (or Generate >
  Settings > Install Pixal3D…). It adds the Pixal3D ComfyUI nodes
  (Saganaki22/Pixal3D-ComfyUI, whose Windows-ready copy of the Pixal3D code
  the engine uses), the missing Python and CUDA packages (it never replaces
  torch or transformers), signs you in to Hugging Face (Pixal3D's download
  asks you to accept its terms once) and downloads the model (~12 GB, the
  multi-view weights are skipped for now) and the DINOv3 picture reader into
  `ComfyUI\models\Pixal3D`.
- **For TRELLIS.2:** the ComfyUI-Trellis2 nodes (Easy-Install:
  `Add-Ons\Trellis2 (requires Torch 2.8.0+cu128).bat`).
- The models in `ai/models.json`. The Generate tab's Settings can download them.

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

`mesh` uses Pixal3D (set `THREADSMITH_TRELLIS=1` for TRELLIS.2) and prints a
line for each live preview.

## Licenses

FLUX.2 [klein] 4B: Apache 2.0. Pixal3D (code and weights): MIT. Pixal3D-ComfyUI
nodes: MIT. DINOv3 (Pixal3D's and TRELLIS.2's picture reader): Meta's DINOv3
License. TRELLIS.2: MIT. ComfyUI-Trellis2 nodes: MIT. Users download the
models themselves; ThreadSmith doesn't ship them.
ComfyUI is GPL-3.0; ThreadSmith only talks to it over HTTP as a separate program.
