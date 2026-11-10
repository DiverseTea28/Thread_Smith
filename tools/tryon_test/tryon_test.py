"""Quick test of "generate on the body": can FLUX.2 [klein] dress ThreadSmith's
own body renders cleanly, from six sides, with the SAME clothes in every view?

ThreadSmith rendered the body from the front, back, left, right, top and
bottom with a known camera (input/body_<side>.png + input/camera.json).
For each test garment:
  1. FLUX dresses the front view from the text.
  2. Every other view is dressed with TWO pictures: that view of the body, and
     the dressed front as the example of what the clothes look like, so all
     six views show the same garment.
Saved for each garment and side:
    <garment>_<side>.png        the dressed body
    <garment>_<side>_mask.png   where the picture changed = where the clothes are
    sheet.png                   one row per garment: body, then the six dressed views

Run "Run try-on test.bat" (same folder). Uses ComfyUI at 127.0.0.1:8188 and
starts it hidden if it isn't running. Standard library + Pillow (ComfyUI's
Python has both).
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent  # ThreadSmith folder
COMFY_ROOT = Path(r"C:\ComfyUI-Easy-Install\ComfyUI-Easy-Install")
BASE = "http://127.0.0.1:8188"

GARMENTS = {
    "denim_jacket": "a light blue denim jacket, open at the front, sleeves down to the wrists",
    "red_hoodie": "a plain red pullover hoodie with the hood down",
    "cargo_pants": "black cargo pants with side pockets",
    "sneakers": "white low-top sneakers",
    "beanie": "a dark green knitted beanie",
}
# "top"/"bottom" (straight down/up) are rendered too, but FLUX can't draw them
# (it draws a high-angle or back view instead), so the test uses "above"
# (front from 55° up) and "below" (front from 35° down). Add --all-views to try them anyway.
VIEWS = ["front", "back", "left", "right", "above", "below"]
if "--all-views" in sys.argv:
    VIEWS += ["top", "bottom"]
SEEN_FROM = {
    "front": "from the front",
    "back": "from behind",
    "left": "exactly side-on from their left side (in profile, facing the left edge of the picture)",
    "right": "exactly side-on from their right side (in profile, facing the right edge of the picture)",
    "top": "from directly above (looking straight down at the top of the head and the shoulders)",
    "bottom": "from directly below (looking straight up at the soles of the feet)",
    "above": "from the front and high above (camera looking down at about 55 degrees, so the top of the head and shoulders show and the legs look shorter)",
    "below": "from the front and low down (camera near the floor looking up at about 35 degrees, so the feet and legs look large and the head small)",
}
FRONT_PROMPT = (
    "This is a smooth grey 3D mannequin on a flat plain light grey background. Put {garment} on the mannequin, "
    "fitted naturally, with realistic fabric, seams and folds. Add ONLY that one item: no other clothing, no "
    "underwear, no shoes or hats unless asked; everything else stays bare smooth grey mannequin. Keep the exact "
    "same pose, proportions, camera, framing, flat lighting and flat plain light grey background with no "
    "texture. Seen from the front."
)
VIEW_PROMPT = (
    "Image 1 is a smooth grey 3D mannequin seen {seen}, on a flat plain light grey background. "
    "{refs} Put that same {garment} on the mannequin in image 1, exactly as it would look {seen}: same "
    "colour, material, fit and details. The output must show the mannequin {seen}, with exactly the pose, "
    "silhouette, camera and framing of image 1; do not turn it to face the camera. Add ONLY that one item; "
    "everything else stays bare smooth grey mannequin. Flat plain light grey background with no texture."
)
BARE_PROMPT = (
    "This is a smooth grey 3D mannequin seen {seen}, on a flat plain light grey background. Show it exactly as "
    "it is: no clothing, same pose, silhouette, camera and framing, flat plain light grey background with no texture."
)
REFS_TEXT = {
    1: "Image 2 shows the same mannequin from the front, wearing {garment}.",
    2: "Images 2 and 3 show the same mannequin from the front and from behind, wearing {garment}.",
}
MAX_TRIES = 3

def call(path: str, data: bytes | None = None, headers: dict | None = None, timeout: int = 30) -> bytes:
    req = urllib.request.Request(BASE + path, data=data, headers=headers or {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read()


def up() -> bool:
    try:
        call("/system_stats", timeout=3)
        return True
    except Exception:
        return False


def start_comfy() -> subprocess.Popen | None:
    if up():
        return None
    py = COMFY_ROOT / "python_embeded" / "python.exe"
    main = COMFY_ROOT / "ComfyUI" / "main.py"
    print("Starting ComfyUI (hidden)... this takes a minute")
    log = open(HERE / "comfyui_log.txt", "w", encoding="utf-8")
    flags = 0x08000000 if sys.platform == "win32" else 0  # no window
    p = subprocess.Popen(
        [str(py), "-I", str(main), "--windows-standalone-build", "--disable-auto-launch", "--listen", "127.0.0.1", "--port", "8188"],
        cwd=str(COMFY_ROOT), stdout=log, stderr=log, creationflags=flags,
    )
    for _ in range(600):
        if up():
            return p
        if p.poll() is not None:
            raise SystemExit("ComfyUI closed while starting. See comfyui_log.txt")
        time.sleep(0.5)
    raise SystemExit("ComfyUI took too long to start.")


def upload(path: Path) -> str:
    boundary = uuid.uuid4().hex
    body = (
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{path.name}\"\r\n"
        "Content-Type: image/png\r\n\r\n"
    ).encode() + path.read_bytes() + f"\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n--{boundary}--\r\n".encode()
    r = json.loads(call("/upload/image", body, {"Content-Type": f"multipart/form-data; boundary={boundary}"}))
    return (r.get("subfolder") + "/" if r.get("subfolder") else "") + r["name"]


def add_references(workflow: dict, image_names: list[str]) -> dict:
    """Give the FLUX edit workflow more pictures to look at (the dressed
    front/back), chained after the first one."""
    w = dict(workflow)
    pos, neg = ["24", 0], ["25", 0]
    for i, name in enumerate(image_names):
        n = 30 + i * 10
        w[str(n)] = {"class_type": "LoadImage", "inputs": {"image": name}}
        w[str(n + 1)] = {"class_type": "ImageScaleToTotalPixels",
                         "inputs": {"image": [str(n), 0], "upscale_method": "nearest-exact", "megapixels": 1.0, "resolution_steps": 1}}
        w[str(n + 2)] = {"class_type": "VAEEncode", "inputs": {"pixels": [str(n + 1), 0], "vae": ["3", 0]}}
        w[str(n + 3)] = {"class_type": "ReferenceLatent", "inputs": {"conditioning": pos, "latent": [str(n + 2), 0]}}
        w[str(n + 4)] = {"class_type": "ReferenceLatent", "inputs": {"conditioning": neg, "latent": [str(n + 2), 0]}}
        pos, neg = [str(n + 3), 0], [str(n + 4), 0]
    w["6"] = {**w["6"], "inputs": {**w["6"]["inputs"], "positive": pos, "negative": neg}}
    return w


def fill(template, values: dict):
    if isinstance(template, dict):
        return {k: fill(v, values) for k, v in template.items() if not k.startswith("_")}
    if isinstance(template, list):
        return [fill(v, values) for v in template]
    if isinstance(template, str) and template.startswith("$") and template[1:] in values:
        return values[template[1:]]
    return template


def run(workflow: dict, out_node: str) -> bytes:
    pid = json.loads(call("/prompt", json.dumps({"prompt": workflow, "client_id": "threadsmith-test"}).encode(),
                          {"Content-Type": "application/json"}))["prompt_id"]
    while True:
        time.sleep(1.5)
        hist = json.loads(call(f"/history/{pid}"))
        if pid in hist:
            item = hist[pid]
            status = item.get("status", {})
            if status.get("status_str") == "error":
                raise RuntimeError(json.dumps(status.get("messages", []))[-1500:])
            img = item["outputs"][out_node]["images"][0]
            q = urllib.parse.urlencode({"filename": img["filename"], "subfolder": img.get("subfolder", ""), "type": img.get("type", "output")})
            return call(f"/view?{q}", timeout=120)


def _arr(path):
    import numpy as np
    from PIL import Image
    return np.asarray(Image.open(path).convert("RGB")).astype(np.int16)


def _foreground(img):
    """Pixels that aren't background (background colour taken from the edges)."""
    import numpy as np
    from scipy import ndimage
    border = np.concatenate([img[:8].reshape(-1, 3), img[-8:].reshape(-1, 3), img[:, :8].reshape(-1, 3), img[:, -8:].reshape(-1, 3)])
    bg = np.median(border, axis=0)
    fg = np.abs(img - bg).max(axis=2) > 22
    fg = ndimage.binary_opening(fg, iterations=2)
    fg = ndimage.binary_fill_holes(fg)
    return fg


def _head_match(out, view_png: Path, other_png: Path, sil) -> float:
    """Front and back have the same outline, so compare the head (face or no
    face): >0 when the result's head looks more like this view than the
    opposite one."""
    import numpy as np
    ys, xs = np.nonzero(sil)
    top, bottom = ys.min(), ys.max()
    h = int((bottom - top) * 0.11)
    cx = int(np.median(xs[ys < top + h])) if (ys < top + h).any() else int(xs.mean())
    box = (slice(top, top + h), slice(max(0, cx - h // 2), cx + h // 2))

    def gray(a):
        g = a[box].mean(axis=2).astype(np.float32)
        g = g - g.mean()
        return g / (np.linalg.norm(g) + 1e-6)

    o = gray(out)
    return float((o * gray(_arr(view_png))).sum() - (o * gray(_arr(other_png))).sum())


OPPOSITE = {"front": "back", "back": "front"}


def _head_covered(out, body, sil) -> bool:
    """A hat or hood hides the face, so the face test can't be used."""
    import numpy as np
    ys, xs = np.nonzero(sil)
    top = ys.min()
    h = int((ys.max() - top) * 0.11)
    region = sil[top:top + h]
    changed = np.abs(out[top:top + h] - body[top:top + h]).max(axis=2) > 40
    return (changed & region).sum() > 0.35 * max(1, region.sum())


def check_and_composite(body_png: Path, dressed_png: Path, comp_png: Path, mask_png: Path, baseline_png: Path | None = None) -> tuple[float, str]:
    """Score how well the dressed picture keeps the body's view (1 = perfect),
    and save a clean version: the AI's pixels only where the clothes are,
    the original render everywhere else."""
    import numpy as np
    from PIL import Image
    from scipy import ndimage
    body = _arr(body_png)
    out = _arr(dressed_png)
    if out.shape != body.shape:
        out = np.asarray(Image.fromarray(out.astype(np.uint8)).resize(body.shape[1::-1])).astype(np.int16)
    sil = np.abs(body - body[0, 0]).max(axis=2) > 6           # the bare body
    fg = _foreground(out)                                       # body + clothes in the result
    near = ndimage.binary_dilation(sil, iterations=40)          # room for clothes to stick out
    recall = (sil & fg).sum() / max(1, sil.sum())               # body still where it was?
    spill = (fg & ~near).sum() / max(1, fg.sum())               # stuff far from the body?
    score = float(recall - 2.0 * spill)
    view = body_png.stem.replace("body_", "")
    head = ""
    if view in OPPOSITE and not _head_covered(out, body, sil):
        m = _head_match(out, body_png, body_png.with_name(f"body_{OPPOSITE[view]}.png"), sil)
        head = f", head {'right' if m > 0 else 'WRONG'} way ({m:+.2f})"
        if m <= 0:
            score -= 1.0
    # Clothes mask: compare with FLUX's own picture of the BARE mannequin
    # (same view, same re-lighting), so only the clothes differ.
    if baseline_png is not None and baseline_png.is_file():
        base = _arr(baseline_png)
        if base.shape != out.shape:
            base = np.asarray(Image.fromarray(base.astype(np.uint8)).resize(out.shape[1::-1])).astype(np.int16)
        d = ndimage.gaussian_filter(np.abs(out - base).max(axis=2).astype(np.float32), 1.5)
        unlike = d > 32
    else:
        unlike = np.abs(out - body).max(axis=2) > 30
    mask = fg & near & (unlike | (fg & ~ndimage.binary_dilation(sil, iterations=3)))
    mask = ndimage.binary_opening(mask, iterations=2)
    mask = ndimage.binary_closing(mask, iterations=5)
    mask = ndimage.binary_fill_holes(mask)
    labels, n = ndimage.label(mask)
    if n:
        sizes = ndimage.sum(mask, labels, range(1, n + 1))
        mask = np.isin(labels, [i + 1 for i, sz in enumerate(sizes) if sz > 600])
    Image.fromarray((mask * 255).astype(np.uint8)).save(mask_png)
    # The picture itself is kept as FLUX made it (the engine removes the
    # plain background); the mask is for cutting the body away later.
    Image.fromarray(out.astype(np.uint8)).save(comp_png)
    return score, f"body kept {recall * 100:.0f}%, outside {spill * 100:.0f}%{head}, clothes {mask.mean() * 100:.1f}%"


def main() -> int:
    from PIL import Image

    inp = HERE / "input"
    out = ROOT / "Generated" / "tryon_test" / time.strftime("run_%H%M%S")
    if not (inp / "body_front.png").is_file():
        raise SystemExit(f"Missing {inp / 'body_front.png'}")
    out.mkdir(parents=True, exist_ok=True)
    only = [a for a in sys.argv[1:] if a in GARMENTS]
    for k in list(GARMENTS):
        if only and k not in only:
            del GARMENTS[k]
    template = json.loads((ROOT / "ai" / "workflows" / "image_from_image.json").read_text(encoding="utf-8"))
    out_node = template["_threadsmith"]["output_node"]
    started = start_comfy()
    try:
        # Neutral grey versions of the renders (FLUX handles a plain grey
        # mannequin better than ThreadSmith's bluish viewport colour).
        grey_dir = out / "input"
        grey_dir.mkdir(exist_ok=True)
        for v in VIEWS:
            src = inp / f"body_{v}.png"
            if src.is_file():
                Image.open(src).convert("L").convert("RGB").save(grey_dir / src.name)
        for f in ("camera.json",):
            if (inp / f).is_file():
                (grey_dir / f).write_bytes((inp / f).read_bytes())
        inp = grey_dir
        names = {v: upload(inp / f"body_{v}.png") for v in VIEWS if (inp / f"body_{v}.png").is_file()}
        views = [v for v in VIEWS if v in names]
        # FLUX's picture of the bare mannequin from each view: the baseline the
        # clothes masks are measured against.
        bare: dict[str, Path] = {}
        for view in views:
            values = {
                "IMAGE_MODEL": "flux-2-klein-4b.safetensors", "TEXT_ENCODER": "qwen_3_4b.safetensors",
                "IMAGE_VAE": "flux2-vae.safetensors", "SEED": 7, "STEPS": 4, "REF_IMAGE": names[view],
                "PROMPT": BARE_PROMPT.format(seen=SEEN_FROM[view]),
            }
            bare[view] = out / f"bare_{view}.png"
            bare[view].write_bytes(run(fill(template, values), out_node))
            print(f"bare {view}: done")
        report = []
        rows = []
        for i, (key, garment) in enumerate(GARMENTS.items()):
            row = []
            dressed_refs: dict[str, str] = {}
            for view in views:
                refs = [dressed_refs[v] for v in ("front", "back") if v in dressed_refs and view not in ("front", "back")]
                if view == "back" and "front" in dressed_refs:
                    refs = [dressed_refs["front"]]
                best = None
                for attempt in range(MAX_TRIES):
                    values = {
                        "IMAGE_MODEL": "flux-2-klein-4b.safetensors", "TEXT_ENCODER": "qwen_3_4b.safetensors",
                        "IMAGE_VAE": "flux2-vae.safetensors", "SEED": 1000 + i * 100 + attempt, "STEPS": 4,
                        "REF_IMAGE": names[view],
                    }
                    if view == "front":
                        values["PROMPT"] = FRONT_PROMPT.format(garment=garment)
                        wf = fill(template, values)
                    else:
                        values["PROMPT"] = VIEW_PROMPT.format(
                            garment=garment, seen=SEEN_FROM[view], refs=REFS_TEXT[len(refs)].format(garment=garment))
                        wf = add_references(fill(template, values), refs)
                    t0 = time.time()
                    png = run(wf, out_node)
                    raw = out / f"{key}_{view}_try{attempt + 1}.png"
                    raw.write_bytes(png)
                    comp = out / f"{key}_{view}_try{attempt + 1}_final.png"
                    score, why = check_and_composite(inp / f"body_{view}.png", raw, comp, comp.with_name(raw.stem + "_mask.png"), bare[view])
                    print(f"{key} {view} try {attempt + 1}: score {score:.2f} ({why}), {time.time() - t0:.0f}s")
                    if best is None or score > best[0]:
                        best = (score, raw, comp, why)
                    if score >= 0.85:
                        break
                score, raw, comp, why = best
                final = out / f"{key}_{view}.png"
                final.write_bytes(comp.read_bytes())
                mask_file = comp.with_name(raw.stem + "_mask.png")
                if mask_file.is_file():
                    (out / f"{key}_{view}_mask.png").write_bytes(mask_file.read_bytes())
                (out / f"{key}_{view}_raw.png").write_bytes(raw.read_bytes())
                if view in ("front", "back"):
                    dressed_refs[view] = upload(final)
                report.append(f"{key:14s} {view:7s} score {score:5.2f}  {why}")
                row.append(final)
            rows.append(row)
        (out / "report.txt").write_text("\n".join(report), encoding="utf-8")
        print("\n".join(report))
        # Sheet: one row per garment: bare front body, then each final view.
        cell = 256
        sheet = Image.new("RGB", (cell * (len(views) + 1), cell * len(rows)), (40, 40, 40))
        for r, row in enumerate(rows):
            for c, p in enumerate([inp / "body_front.png"] + row):
                sheet.paste(Image.open(p).convert("RGB").resize((cell, cell)), (c * cell, r * cell))
        sheet.save(out / "sheet.png")
        # Masks sheet: the clothes mask shown in red over each view.
        red = Image.new("RGB", (cell, cell), (255, 0, 0))
        msheet = Image.new("RGB", (cell * len(views), cell * len(rows)), (40, 40, 40))
        for r, row in enumerate(rows):
            for c, p in enumerate(row):
                img = Image.open(p).convert("RGB").resize((cell, cell))
                m = p.with_name(p.stem + "_mask.png")
                if m.is_file():
                    img = Image.composite(Image.blend(img, red, 0.5), img, Image.open(m).convert("L").resize((cell, cell)))
                msheet.paste(img, (c * cell, r * cell))
        msheet.save(out / "masks.png")
        print(f"\nDone. Look at {out / 'sheet.png'}")
    finally:
        if started is not None:
            started.terminate()
    return 0


if __name__ == "__main__":
    sys.exit(main())
