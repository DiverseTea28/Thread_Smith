# ThreadSmith

A free Windows tool for making clothes for MetaHumans: import a body, generate
clothing with local AI, fit it, auto-skin it, and send it back to Unreal 5.8.

**Current stage: Phase 0 (Foundations).** The app can open a model, show it
in a 3D viewport, show its skeleton, and pose bones. No clothing features yet.

## What works now

- Open **FBX, glTF / GLB and OBJ** files (File → Open, or drag a file onto the window).
- Unreal FBX files (Z up, centimeters) are turned the right way up and scaled to meters.
- Skinned meshes with up to 8 bones per vertex (what MetaHuman bodies use).
- 3D viewport: shaded view, wireframe, skeleton, floor grid.
- Camera: **left drag** orbit, **right or middle drag** pan, **scroll** zoom, **F** frame the model.
- Pick a bone in the list (or click a bone dot in the viewport) and rotate it with
  the X / Y / Z sliders. The mesh bends with it. "Reset to bind pose" puts it back.
- The status bar says when it finds a UE5 / MetaHuman skeleton.

## Setup (one time)

1. **Install Rust:** download and run `rustup-init.exe` from <https://rustup.rs>.
   Pick the default options.
2. **Install the C++ build tools** (Rust needs them on Windows, and the FBX
   reader is written in C): install "Visual Studio Build Tools" and tick
   **"Desktop development with C++"**. If you already have Visual Studio with
   C++ for Unreal, you can skip this.

## Run it

Open a terminal (PowerShell) in this folder and run:

```
cargo run --release
```

The first build downloads and compiles everything, so it takes a few minutes.
After that it starts in seconds. The finished program is at
`target\release\threadsmith.exe`; you can run that directly.

To open a file straight away:

```
cargo run --release -- "C:\path\to\MetaHuman_Body.fbx"
```

## Getting a MetaHuman body out of Unreal

In the Content Browser, find the MetaHuman's body skeletal mesh (for example
`m_med_nrw_body`), right-click → **Asset Actions → Export**, and save as FBX.
Open that FBX in ThreadSmith.

## Project layout

| Folder | What's in it |
| --- | --- |
| `crates/ts-core` | Shared types: meshes, skeletons, poses, skinning math |
| `crates/ts-io` | File import (glTF through `gltf`, FBX and OBJ through `ufbx`) |
| `crates/ts-app` | The app window, panels and the wgpu 3D viewport |

Useful extra commands:

- `cargo test --workspace` runs the tests.
- `cargo run -p ts-io --example inspect -- "file.fbx"` prints what the importer
  sees in a file (parts, bones, size) without opening the window. Handy for
  checking MetaHuman exports.

## Next: Phase 1 (fit and skin, no AI yet)

Import a garment file, shell and fit it onto the body, auto-skin it with weight
inpainting, test poses, set poly count and LODs, and export.
