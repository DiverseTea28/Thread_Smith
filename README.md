# ThreadSmith

A free Windows tool for making clothes for MetaHumans: import a body, generate
clothing with local AI, fit it, auto-skin it, and send it back to Unreal 5.8.

**Current stage: Unreal bridge, ThreadSmith side.** You can open a MetaHuman
body, add a clothing file, fit it, auto-skin it, save it to the outfit library
(with a thumbnail), and send it to Unreal. The Unreal plugin that receives it
(and the in-editor wardrobe) is the next step.

## What works now

- Open **FBX, glTF / GLB and OBJ** files (File → Open, or drag a file onto the window).
- Unreal FBX files (Z up, centimeters) are turned the right way up and scaled to meters.
- Skinned meshes with up to 8 bones per vertex (what MetaHuman bodies use).
- 3D viewport: shaded view, wireframe, skeleton, floor grid.
- Camera: **left drag** orbit, **right or middle drag** pan, **scroll** zoom, **F** frame the model.
- Pick a bone in the list (or click a bone dot in the viewport) and rotate it with
  the X / Y / Z sliders. The mesh bends with it. "Reset to bind pose" puts it back.
- The status bar says when it finds a UE5 / MetaHuman skeleton.
- Only LOD0 is shown when a file has LODs (tick the others to see them).
- Click "parent: …" under a bone's name to jump up to its parent bone (handy
  for MetaHuman twist and corrective bones).

### Clothes (new)

1. Open the body first (**File → Open body…**).
2. **File → Add garment…** (or drag a clothing file onto the window).
   - If the file also has the body the garment was made on (a part named
     like "Avatar" or "Body", or a much bigger mesh), ThreadSmith uses it to
     line the garment up with your MetaHuman. It's hidden and marked
     "reference" in the mesh list.
3. **Place** it on the body first (section 1 of the Garment panel):
   - Viewport tools (top-left of the 3D view, or keys): **Move (W)**,
     **Rotate (E)**, **Scale (R)**. Drag an arrow / ring / handle, or the
     centre square to move freely (Move) or change the overall size (Scale).
     **Q** goes back to camera only. Alt + drag always orbits.
   - Or type exact numbers: *Move (cm)*, *Turn (°)*, *Stretch (%)*, *size*.
   - **Grow to clear body** makes a too-small garment just big enough that it
     no longer sinks into the body.
   - Moving a garment after fitting undoes the fit (fit it again).
4. **Fit to body** (section 2) shrink wraps it on. With a reference body it
   keeps the garment's distance from its original body; without one it pulls
   loose cloth in by *Tightness*. Then it pushes out anything inside the skin.
   - *Gap from skin*: smallest space left between cloth and skin.
   - *Keep shape*: higher keeps the garment's own shape, lower hugs the body.
   - *Reset* puts the garment back to how it was added (placement too).
5. **Sculpt** to touch it up (section 3): **Grab (G)** pulls a soft patch of
   cloth, **Relax (T)** smooths wrinkles and bumps, **Inflate (I)** pushes
   cloth out (hold Ctrl to push in). **[** and **]** change the brush size.
   *Keep outside the skin* pops anything pushed into the body back out when
   you let go. Sculpting needs the bind pose.
6. **Auto skin** copies the MetaHuman's skin weights onto the garment, then
   fills in loose parts smoothly (Epic's "weight inpainting" method).
   - *Copy distance*: closer than this copies weights directly.
   - *Rigid*: the whole piece follows one bone (helmets, buckles).
7. Pose bones on the right to test it. Fitting and skinning always work on
   the bind pose, whatever pose is showing.

**Ctrl+Z** undoes placing, fitting, sculpting and skinning (last 40 steps).

### Animations (new)

Test the clothes in motion, like MetaTailor:

1. **Add animations…** (Animation panel on the right, or the File menu) and
   pick Mixamo FBX files. Download them from Mixamo as **FBX, With Skin,
   30 fps** (any Mixamo character works). They are copied into
   `Documents\ThreadSmith\Library\Animations\`, so they're there next time.
2. Click one to play it on the body. ThreadSmith retargets it from the
   Mixamo skeleton (T-pose) onto the MetaHuman skeleton (A-pose).
   *In place* keeps the body over the same spot; *Loop*, *speed* and the
   time slider work as you'd expect. **Stop** goes back to the rest pose.
3. **Show clipping** puts red marks wherever a skinned garment pokes
   through the skin on the frame showing. **Scan whole animation** checks
   every other frame and lists the worst moments; click one to jump there.
4. To fix clipping: Stop, then use the Sculpt brushes, a bigger gap, or
   less tightness (fit and skin again), and scan again.

Only skinned garments follow the body, so Auto skin before testing.

### Save and send (new)

6. **3. Save and send** in the Garment panel: give the outfit a name and type.
   - **Save to library** writes it to `Documents\ThreadSmith\Library\Outfits\`
     as `outfit.glb` (skinned on the body's skeleton), `outfit.json` and a
     `thumbnail.png`. The left panel lists saved outfits.
   - **Send to Unreal** saves it, then hands it to the Unreal plugin. It's
     greyed out until the status bar says **Unreal connected**.
7. The `.glb` also opens in Blender, rigged to the same bones.

### Unreal bridge

**In Unreal:** open your character's Blueprint and click **Send to
ThreadSmith** in its toolbar (or select the character in the level and use
**To ThreadSmith**). The body and head are sent. When you **Send to Unreal**
from ThreadSmith with *Put it on the character* ticked, the outfit is added
to that Blueprint as a component (like MetaTailor), saved, and every copy of
the character wears it. Sending a newer version of the same outfit replaces
the old one. To take it off, delete the component in the Blueprint.
The **ThreadSmith Wardrobe** (toolbar button) lists your library; **Add to
character** (or double-click) adds an outfit to the open Blueprint.

To update the plugin after changes: run `unreal\UpdatePlugin.bat`. It
builds the plugin, waits for you to close Unreal, installs it, and opens
your project again.

ThreadSmith listens on `127.0.0.1:47830` for bodies sent from Unreal, and
looks for the plugin on `127.0.0.1:47831`. Details for plugin work:
`docs/bridge-protocol.md`. Start ThreadSmith with `-` instead of a body to
wait for one from Unreal: `cargo run --release -- - "C:\path\to\TShirt.fbx"`.

You can open both at once from the command line:

```
cargo run --release -- "C:\path\to\Body.fbx" "C:\path\to\TShirt.fbx"
```

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
| `crates/ts-fit` | Garment fitting and auto skinning (closest-point search, fit, push-out, weight transfer) |
| `crates/ts-library` | Outfit library folder (outfit.json + outfit.glb + thumbnail.png) |
| `docs/bridge-protocol.md` | Messages between ThreadSmith and the Unreal plugin |
| `crates/ts-app` | The app window, panels and the wgpu 3D viewport |

Useful extra commands:

- `cargo test --workspace` runs the tests.
- `cargo run -p ts-io --example inspect -- "file.fbx"` prints what the importer
  sees in a file (parts, bones, size) without opening the window. Handy for
  checking MetaHuman exports.
- `cargo run --release -p ts-io --example export_demo -- body.fbx garment.fbx out.glb`
  fits, skins, exports a GLB, loads it back and checks it matches.
- `cargo run --release -p ts-fit --example fit_demo -- body.fbx garment.fbx`
  fits and skins without the window and prints how many garment vertices end
  up inside the body (before, after fitting, and in a test pose).

## Next

- **ThreadSmith Bridge plugin for UE 5.8** (C++, built once, works in
  Blueprint-only projects): "Send to ThreadSmith" button, outfit import, and
  the Wardrobe tab.
- Poly count slider and LOD chain.
- Hide body parts that are fully covered by clothes.
- "Shell" step for AI-made garments that are solid blobs.
