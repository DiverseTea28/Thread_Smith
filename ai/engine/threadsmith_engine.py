"""ThreadSmith Engine: runs the 3D model (Pixal3D) for ThreadSmith.

ThreadSmith starts this script hidden, with the Python from the ComfyUI
install (it already has torch and the CUDA packages). The script connects back
to ThreadSmith over a local TCP connection (127.0.0.1 only) and then waits
for jobs. While a model is being made it sends live previews: a cloud of
cubes while the rough shape forms, then grey meshes, then the finished GLB.

Messages (both ways) are:
    u32 header length | JSON header | u32 data length | data
with little-endian lengths. The JSON header always has a "type".

App -> engine:
    status                      what's installed, graphics card, memory
    generate {...}              make a 3D model from a picture
    cancel                      stop the running job
    unload                      free the graphics card memory
    shutdown                    quit

Engine -> app:
    hello {protocol}
    status {models, gpu, vram_gb, low_vram, loaded, problem}
    progress {stage, label, step, total, fraction}
    preview_voxels {count, size}       + f32 xyz centres (Y-up, like the GLB)
    preview_mesh {vertices, faces}     + f32 xyz per vertex, then u32 per face corner
    log {text}
    done {glb_path, seconds}
    error {message}

Test mode: run with --fake (or THREADSMITH_ENGINE_FAKE=1) and the engine
pretends to generate (a growing ball of cubes, then a ball mesh, then a GLB)
without a graphics card or any AI packages. ThreadSmith's side was tested
this way.

Only the Python standard library is needed to start; torch, numpy and the
Pixal3D code are imported when the first real job (or status check) runs.
"""

from __future__ import annotations

import argparse
import importlib
import importlib.util
import json
import math
import os
import queue
import socket
import struct
import sys
import threading
import time
import traceback
from pathlib import Path

PROTOCOL = 1
ENGINE_VERSION = "0.2.0"

# The Pixal3D files ThreadSmith expects (shared with the Pixal3D ComfyUI nodes).
PIXAL3D_REPO = "TencentARC/Pixal3D"
DINO_REPO = "camenduru/dinov3-vitl16-pretrain-lvd1689m"
NODES_FOLDER = "Pixal3D-ComfyUI"

# Pixal3D's voxel grid is a box from -0.5 to 0.5 with +y up (it's lined up
# with the picture). Its GLB export (a Z/Y swap, then its own turn) adds up to
# a half turn around y, so previews get the same half turn and line up with
# the final model.
ROT = ((-1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, -1.0))


class Cancelled(Exception):
    pass


# ---------------------------------------------------------------------------
# Connection
# ---------------------------------------------------------------------------


class Connection:
    def __init__(self, port: int):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=30)
        self.sock.settimeout(None)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.lock = threading.Lock()

    def send(self, header: dict, data: bytes = b"") -> None:
        head = json.dumps(header).encode("utf-8")
        packet = struct.pack("<I", len(head)) + head + struct.pack("<I", len(data))
        with self.lock:
            self.sock.sendall(packet)
            if data:
                self.sock.sendall(data)

    def _read_exact(self, n: int) -> bytes:
        buf = bytearray()
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("ThreadSmith closed the connection")
            buf.extend(chunk)
        return bytes(buf)

    def recv(self) -> tuple[dict, bytes]:
        (head_len,) = struct.unpack("<I", self._read_exact(4))
        header = json.loads(self._read_exact(head_len).decode("utf-8"))
        (data_len,) = struct.unpack("<I", self._read_exact(4))
        data = self._read_exact(data_len) if data_len else b""
        return header, data


class Reporter:
    """Sends progress to the app and checks for Stop between steps."""

    # Share of the whole job for each stage (adds up to 1).
    WEIGHTS = {
        "prepare": 0.04,
        "structure": 0.10,
        "shape": 0.22,
        "detail": 0.24,
        "texture": 0.20,
        "decode": 0.06,
        "bake": 0.14,
    }
    ORDER = ["prepare", "structure", "shape", "detail", "texture", "decode", "bake"]

    def __init__(self, conn: Connection, cancel: threading.Event, live: bool):
        self.conn = conn
        self.cancel = cancel
        self.live = live

    def check(self) -> None:
        if self.cancel.is_set():
            raise Cancelled()

    def progress(self, stage: str, label: str, step: int = 0, total: int = 0) -> None:
        self.check()
        before = sum(self.WEIGHTS[s] for s in self.ORDER[: self.ORDER.index(stage)])
        within = (step / total) if total else 0.0
        fraction = before + self.WEIGHTS[stage] * within
        self.conn.send(
            {"type": "progress", "stage": stage, "label": label, "step": step, "total": total, "fraction": round(fraction, 4)}
        )

    def log(self, text: str) -> None:
        log(text)
        self.conn.send({"type": "log", "text": text})

    def voxels(self, centres, size: float) -> None:
        """centres: flat list/array of x, y, z (already turned Y-up)."""
        if not self.live:
            return
        data = _f32_bytes(centres)
        self.conn.send({"type": "preview_voxels", "count": len(data) // 12, "size": size}, data)

    def mesh(self, vertices, faces) -> None:
        """vertices: flat x, y, z (Y-up). faces: flat vertex indices."""
        if not self.live:
            return
        vdata = _f32_bytes(vertices)
        fdata = _u32_bytes(faces)
        self.conn.send({"type": "preview_mesh", "vertices": len(vdata) // 12, "faces": len(fdata) // 12}, vdata + fdata)


def _f32_bytes(values) -> bytes:
    try:
        import numpy as np

        return np.ascontiguousarray(np.asarray(values, dtype="<f4")).tobytes()
    except ImportError:
        return struct.pack(f"<{len(values)}f", *values)


def _u32_bytes(values) -> bytes:
    try:
        import numpy as np

        return np.ascontiguousarray(np.asarray(values, dtype="<u4")).tobytes()
    except ImportError:
        return struct.pack(f"<{len(values)}I", *values)


# ---------------------------------------------------------------------------
# Log file
# ---------------------------------------------------------------------------

_LOG_FILE = None


def log(text: str) -> None:
    line = f"[{time.strftime('%H:%M:%S')}] {text}"
    print(line, flush=True)
    if _LOG_FILE is not None:
        try:
            _LOG_FILE.write(line + "\n")
            _LOG_FILE.flush()
        except Exception:
            pass


# ---------------------------------------------------------------------------
# Pictures: background removal
# ---------------------------------------------------------------------------


def load_cutout(path: str):
    """Open a picture and give it a see-through background (RGBA).

    1. A picture that already has see-through parts is used as is.
    2. A plain light background (what FLUX makes for "ghost mannequin"
       pictures) is removed by flooding in from the edges, so light parts
       *inside* the garment (the inside of a collar) are kept.
    3. Anything else goes through the `rembg` package, if it's installed.
    """
    import numpy as np
    from PIL import Image

    img = Image.open(path)
    img.load()
    if img.mode in ("RGBA", "LA") or (img.mode == "P" and "transparency" in img.info):
        rgba = img.convert("RGBA")
        alpha = np.asarray(rgba)[:, :, 3]
        if (alpha < 250).mean() > 0.02:
            return rgba, "kept the picture's own see-through background"
    rgb = img.convert("RGB")
    arr = np.asarray(rgb).astype(np.int16)
    border = np.concatenate([arr[0], arr[-1], arr[:, 0], arr[:, -1]])
    bg = np.median(border, axis=0)
    border_close = (np.abs(border - bg).max(axis=1) < 28).mean()
    if border_close > 0.85 and bg.min() > 170:
        mask = _flood_background(arr, bg, tolerance=30)
        if 0.05 < (~mask).mean() < 0.97:
            alpha = _soften((~mask).astype(np.float32))
            rgba = np.dstack([np.asarray(rgb), (alpha * 255).astype(np.uint8)])
            return Image.fromarray(rgba, "RGBA"), "removed the plain background"
    try:
        from rembg import remove

        return remove(rgb).convert("RGBA"), "removed the background with rembg"
    except Exception as exc:  # noqa: BLE001
        raise RuntimeError(
            "Couldn't remove the picture's background. Use a picture with a plain light background "
            f"or a see-through PNG. ({type(exc).__name__}: {exc})"
        ) from exc


def _flood_background(arr, bg, tolerance: int):
    """True where a pixel is background: close to `bg` and connected to the edge."""
    import numpy as np

    close = np.abs(arr - bg).max(axis=2) < tolerance
    try:
        from scipy import ndimage

        labels, _ = ndimage.label(close)
        edge = np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))
        edge = edge[edge != 0]
        return np.isin(labels, edge)
    except ImportError:
        # Slow fallback without scipy: grow from the edges.
        mask = np.zeros_like(close)
        mask[0], mask[-1], mask[:, 0], mask[:, -1] = close[0], close[-1], close[:, 0], close[:, -1]
        while True:
            grown = mask.copy()
            grown[1:] |= mask[:-1]
            grown[:-1] |= mask[1:]
            grown[:, 1:] |= mask[:, :-1]
            grown[:, :-1] |= mask[:, 1:]
            grown &= close
            if (grown == mask).all():
                return mask
            mask = grown


def _soften(alpha):
    """Blur the cut edge by about a pixel so it isn't jagged."""
    try:
        from scipy import ndimage

        return ndimage.gaussian_filter(alpha, 0.8).clip(0, 1)
    except ImportError:
        return alpha


# ---------------------------------------------------------------------------
# Models
# ---------------------------------------------------------------------------


def comfy_dirs(comfy_root: str | None) -> tuple[Path | None, Path | None]:
    """(ComfyUI folder, its models folder) inside an install folder."""
    if not comfy_root:
        return None, None
    root = Path(comfy_root)
    for d in (root / "ComfyUI", root):
        if (d / "main.py").is_file():
            return d, d / "models"
    return None, None


class FakeModel:
    """Pretends to generate, for testing ThreadSmith without a graphics card."""

    name = "fake"

    def __init__(self, comfy_root):
        self.loaded = False

    def status(self) -> dict:
        return {
            "models": {"pixal3d": {"installed": True, "problem": None}},
            "gpu": "Test mode (no graphics card)",
            "vram_gb": 0.0,
            "low_vram": False,
            "loaded": self.loaded,
            "problem": None,
        }

    def unload(self):
        self.loaded = False

    def reset_memory(self):
        pass

    def generate(self, job: dict, rep: Reporter) -> str:
        speed = float(os.environ.get("THREADSMITH_ENGINE_FAKE_SPEED", "1"))
        nap = lambda s: time.sleep(s / max(speed, 0.01))  # noqa: E731
        rep.progress("prepare", "Getting the picture ready")
        if not Path(job["image"]).is_file():
            raise RuntimeError(f"The picture {job['image']} doesn't exist.")
        nap(0.3)
        self.loaded = True
        steps = 8
        res = 24
        for step in range(1, steps + 1):
            rep.progress("structure", "Rough shape", step, steps)
            radius = 0.15 + 0.3 * step / steps
            pts = []
            for x in range(res):
                for y in range(res):
                    for z in range(res):
                        c = [(i + 0.5) / res - 0.5 for i in (x, y, z)]
                        r = math.sqrt(sum(v * v for v in c))
                        if radius - 1.5 / res < r < radius:
                            pts.extend(c)
            rep.voxels(pts, 1.0 / res)
            nap(0.15)
        for stage, label in (("shape", "Shape"), ("detail", "Fine detail"), ("texture", "Colours")):
            for step in range(1, steps + 1):
                rep.progress(stage, label, step, steps)
                nap(0.08)
            if stage == "shape":
                v, f = _uv_sphere(0.45, 16, 24)
                rep.mesh(v, f)
        rep.progress("decode", "Building the mesh")
        v, f = _uv_sphere(0.45, 48, 72)
        rep.mesh(v, f)
        nap(0.2)
        rep.progress("bake", "Baking the texture")
        out = Path(job["out_dir"]) / f"garment_{time.strftime('%Y%m%d_%H%M%S')}.glb"
        out.parent.mkdir(parents=True, exist_ok=True)
        _write_sphere_glb(out, 0.45, 24, 36)
        nap(0.2)
        return str(out)


def _uv_sphere(r: float, rings: int, segs: int):
    verts, faces = [], []
    for i in range(rings + 1):
        th = math.pi * i / rings
        for j in range(segs):
            ph = 2 * math.pi * j / segs
            verts.extend([r * math.sin(th) * math.cos(ph), r * math.cos(th), r * math.sin(th) * math.sin(ph)])
    for i in range(rings):
        for j in range(segs):
            a = i * segs + j
            b = i * segs + (j + 1) % segs
            c = a + segs
            d = b + segs
            faces.extend([a, c, b, b, c, d])
    return verts, faces


def _write_sphere_glb(path: Path, r: float, rings: int, segs: int) -> None:
    """A tiny valid GLB (one grey sphere), using only the standard library."""
    verts, faces = _uv_sphere(r, rings, segs)
    vbytes = struct.pack(f"<{len(verts)}f", *verts)
    ibytes = struct.pack(f"<{len(faces)}I", *faces)
    xs, ys, zs = verts[0::3], verts[1::3], verts[2::3]
    gltf = {
        "asset": {"version": "2.0", "generator": "ThreadSmith Engine (test mode)"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "garment"}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [0.6, 0.6, 0.62, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.8}}],
        "buffers": [{"byteLength": len(vbytes) + len(ibytes)}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": len(vbytes), "target": 34962},
            {"buffer": 0, "byteOffset": len(vbytes), "byteLength": len(ibytes), "target": 34963},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": len(verts) // 3, "type": "VEC3",
             "min": [min(xs), min(ys), min(zs)], "max": [max(xs), max(ys), max(zs)]},
            {"bufferView": 1, "componentType": 5125, "count": len(faces), "type": "SCALAR"},
        ],
    }
    js = json.dumps(gltf).encode("utf-8")
    js += b" " * ((4 - len(js) % 4) % 4)
    binary = vbytes + ibytes
    binary += b"\0" * ((4 - len(binary) % 4) % 4)
    total = 12 + 8 + len(js) + 8 + len(binary)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(js), 0x4E4F534A) + js)
        f.write(struct.pack("<II", len(binary), 0x004E4942) + binary)


class Pixal3DModel:
    """Pixal3D (Tencent ARC, MIT licence), run directly with live previews.

    The Pixal3D code comes from the Pixal3D ComfyUI nodes
    (ComfyUI\\custom_nodes\\Pixal3D-ComfyUI), which carry Windows fixes and a
    fallback for when NATTEN has no CUDA kernels. The model files are the same
    ones those nodes use: ComfyUI\\models\\Pixal3D\\...
    """

    name = "pixal3d"

    # Same as Pixal3D's own inference.py.
    IMAGE_COND_CONFIGS = {
        "ss": {"image_size": 512, "grid_resolution": 16},
        "shape_512": {"image_size": 512, "grid_resolution": 32, "use_naf_upsample": True, "naf_target_size": 512},
        "shape_1024": {"image_size": 1024, "grid_resolution": 64, "use_naf_upsample": True, "naf_target_size": 512},
        "tex_1024": {"image_size": 1024, "grid_resolution": 64, "use_naf_upsample": True, "naf_target_size": 1024},
    }

    def __init__(self, comfy_root: str | None):
        self.comfy_root = comfy_root
        self.comfy_dir, self.models_dir = comfy_dirs(comfy_root)
        self.pipeline = None
        self.low_vram = True
        self.postprocess = None

    # ---- Finding things --------------------------------------------------

    def nodes_dir(self) -> Path | None:
        if self.comfy_dir is None:
            return None
        custom = self.comfy_dir / "custom_nodes"
        for name in (NODES_FOLDER, NODES_FOLDER.lower(), "Pixal3D_ComfyUI", "pixal3d-comfyui"):
            d = custom / name
            if (d / "pixal3d" / "pipelines").is_dir():
                return d
        if custom.is_dir():
            for d in custom.iterdir():
                if (d / "pixal3d" / "pipelines" / "pixal3d_image_to_3d.py").is_file() and (d / "pixal3d_comfy").is_dir():
                    return d
        return None

    def model_dir(self) -> Path | None:
        return self._find_snapshot(PIXAL3D_REPO, lambda d: not self._missing_files(d))

    def dino_dir(self) -> Path | None:
        return self._find_snapshot(DINO_REPO, lambda d: (d / "config.json").is_file())

    def _find_snapshot(self, repo: str, ok) -> Path | None:
        if self.models_dir is None:
            return None
        root = self.models_dir / "Pixal3D"
        names = [repo.replace("/", "_"), repo.split("/")[-1], "models--" + repo.replace("/", "--")]
        for name in names:
            d = root / name
            candidates = [d]
            snaps = d / "snapshots"
            if snaps.is_dir():
                candidates.extend(sorted(snaps.iterdir(), key=lambda p: p.stat().st_mtime, reverse=True))
            for c in candidates:
                if c.is_dir() and ok(c):
                    return c
        return None

    @staticmethod
    def _missing_files(model_dir: Path) -> list[str]:
        pipeline_file = model_dir / "pipeline.json"
        if not pipeline_file.is_file():
            return ["pipeline.json"]
        try:
            data = json.loads(pipeline_file.read_text(encoding="utf-8"))
        except Exception:
            return ["pipeline.json"]
        missing = []
        for rel in data.get("args", {}).get("models", {}).values():
            for suffix in (".json", ".safetensors"):
                if not (model_dir / f"{rel}{suffix}").is_file():
                    missing.append(f"{rel}{suffix}")
        return missing

    def problems(self) -> list[str]:
        """Plain-language list of what's missing (empty when ready)."""
        out = []
        if self.comfy_dir is None:
            out.append("ComfyUI wasn't found. Set its folder in ThreadSmith's generator settings.")
            return out
        if self.nodes_dir() is None:
            out.append("The Pixal3D nodes aren't installed. Run ai\\setup\\Install Pixal3D.bat.")
        if self.model_dir() is None:
            out.append("The Pixal3D model files aren't downloaded. Run ai\\setup\\Install Pixal3D.bat.")
        if self.dino_dir() is None:
            out.append("The DINOv3 helper model isn't downloaded. Run ai\\setup\\Install Pixal3D.bat.")
        for group in (("flex_gemm_ap", "flex_gemm"), ("cumesh_vb", "cumesh"), ("o_voxel_vb_ap", "o_voxel"), ("drtk",)):
            if not any(importlib.util.find_spec(n) is not None for n in group):
                out.append(f"The Python package {group[-1]} is missing. Run ai\\setup\\Install Pixal3D.bat.")
        if importlib.util.find_spec("flash_attn") is None and importlib.util.find_spec("flash_attn_interface") is None:
            out.append("FlashAttention (flash_attn) is missing. Pixal3D needs it. Run ai\\setup\\Install Pixal3D.bat.")
        return out

    def status(self) -> dict:
        gpu, vram = gpu_info()
        problems = self.problems()
        return {
            "models": {"pixal3d": {"installed": not problems, "problem": problems[0] if problems else None}},
            "gpu": gpu,
            "vram_gb": vram,
            "low_vram": vram < 22.0,
            "loaded": self.pipeline is not None,
            "problem": problems[0] if problems else None,
            "problems": problems,
        }

    # ---- Loading -----------------------------------------------------------

    def _prepare_imports(self) -> None:
        nodes = self.nodes_dir()
        if nodes is None:
            raise RuntimeError("The Pixal3D nodes aren't installed. Run ai\\setup\\Install Pixal3D.bat.")
        if str(nodes) not in sys.path:
            sys.path.insert(0, str(nodes))

        os.environ.setdefault("PYTORCH_CUDA_ALLOC_CONF", "expandable_segments:True")
        os.environ.setdefault("OPENCV_IO_ENABLE_OPENEXR", "1")
        os.environ.setdefault("FLEX_GEMM_AUTOTUNE_CACHE_PATH", str(nodes / "autotune_cache.json"))
        if importlib.util.find_spec("flash_attn_interface") is not None:
            backend = "flash_attn_3"
        elif importlib.util.find_spec("flash_attn") is not None:
            backend = "flash_attn"
        else:
            raise RuntimeError("FlashAttention (flash_attn) is missing. Pixal3D needs it. Run ai\\setup\\Install Pixal3D.bat.")
        os.environ["ATTN_BACKEND"] = backend
        os.environ["SPARSE_ATTN_BACKEND"] = backend

        # torch.hub cache for the NAF helper next to the other Pixal3D files.
        hub = self.models_dir / "Pixal3D" / "torch_hub"
        hub.mkdir(parents=True, exist_ok=True)
        os.environ.setdefault("TORCH_HOME", str(hub))
        import torch

        try:
            torch.hub.set_dir(str(hub / "hub"))
        except Exception:
            pass

        # The CUDA packages come in two builds with different names. Pixal3D's
        # code imports the plain names, so point those at whichever is here.
        def first(names):
            for n in names:
                try:
                    return importlib.import_module(n), n
                except Exception:
                    continue
            raise ImportError(" or ".join(names) + " isn't installed")

        cumesh, cm_name = first(("cumesh_vb", "cumesh"))
        sys.modules.setdefault("cumesh", cumesh)
        sys.modules.setdefault("cumesh.bvh", importlib.import_module(f"{cm_name}.bvh"))
        flex, fg_name = first(("flex_gemm_ap", "flex_gemm"))
        sys.modules.setdefault("flex_gemm", flex)
        for sub in ("ops", "ops.grid_sample", "ops.spconv", "utils", "kernels"):
            try:
                sys.modules.setdefault(f"flex_gemm.{sub}", importlib.import_module(f"{fg_name}.{sub}"))
            except Exception:
                pass
        ovox, ov_name = first(("o_voxel_vb_ap", "o_voxel"))
        sys.modules.setdefault("o_voxel", ovox)
        for sub in ("convert", "io", "serialize"):
            try:
                sys.modules.setdefault(f"o_voxel.{sub}", importlib.import_module(f"{ov_name}.{sub}"))
            except Exception:
                pass

        # The nodes' GLB export (uses drtk, works on Windows). Loaded from its
        # file because the nodes' package itself needs ComfyUI to import.
        spec = importlib.util.spec_from_file_location("threadsmith_pixal3d_postprocess", nodes / "pixal3d_comfy" / "postprocess.py")
        post = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(post)
        self.postprocess = post

    def load(self, rep: Reporter | None) -> None:
        if self.pipeline is not None:
            return
        problems = self.problems()
        if problems:
            raise RuntimeError(problems[0])
        if rep:
            rep.progress("prepare", "Loading Pixal3D (only the first time)")
        self._prepare_imports()
        import torch

        if not torch.cuda.is_available():
            raise RuntimeError("PyTorch can't see an NVIDIA graphics card (CUDA). Pixal3D needs one.")
        _, vram = gpu_info()
        self.low_vram = vram < 22.0

        from pixal3d.pipelines import Pixal3DImageTo3DPipeline, rembg
        from pixal3d.trainers.flow_matching.mixins.image_conditioned_proj import DinoV3ProjFeatureExtractor

        # ThreadSmith removes backgrounds itself, so Pixal3D's background
        # remover (the gated RMBG-2.0 model) is never loaded.
        if not hasattr(rembg, "ThreadSmithNoRMBG"):
            class ThreadSmithNoRMBG:
                def __init__(self, **_):
                    pass

                def to(self, _device):
                    return self

                def cuda(self):
                    return self

                def cpu(self):
                    return self

                def __call__(self, _image):
                    raise RuntimeError("Background removal should have happened before Pixal3D.")

            rembg.ThreadSmithNoRMBG = ThreadSmithNoRMBG

        model_dir = self.model_dir()
        data = json.loads((model_dir / "pipeline.json").read_text(encoding="utf-8"))
        data["args"]["rembg_model"] = {"name": "ThreadSmithNoRMBG", "args": {}}
        config_name = "_threadsmith_pipeline.json"
        (model_dir / config_name).write_text(json.dumps(data, indent=2), encoding="utf-8")

        log(f"Loading Pixal3D from {model_dir} (low VRAM: {self.low_vram})")
        pipe = Pixal3DImageTo3DPipeline.from_pretrained(str(model_dir), config_file=config_name)
        pipe.low_vram = self.low_vram

        dino = str(self.dino_dir())
        shared = None
        for attr, key in (
            ("image_cond_model_ss", "ss"),
            ("image_cond_model_shape_512", "shape_512"),
            ("image_cond_model_shape_1024", "shape_1024"),
            ("image_cond_model_tex_1024", "tex_1024"),
        ):
            cfg = dict(self.IMAGE_COND_CONFIGS[key])
            cfg["model_name"] = dino
            if cfg.get("use_naf_upsample"):
                # Without NATTEN's CUDA kernels (the usual case on Windows) NAF
                # can't run; the fallback reuses the low-res features instead.
                cfg["naf_fallback_mode"] = "duplicate_lr"
                cfg["naf_download_if_missing"] = True
                cfg["naf_low_vram"] = self.low_vram
            if shared is not None:
                cfg["shared_dino_model"] = shared
            m = DinoV3ProjFeatureExtractor(**cfg)
            m.eval()
            shared = m.model
            setattr(pipe, attr, m)

        if self.low_vram:
            pipe._device = torch.device("cuda")
        else:
            pipe.cuda()
            for attr in ("image_cond_model_ss", "image_cond_model_shape_512", "image_cond_model_shape_1024", "image_cond_model_tex_1024"):
                getattr(pipe, attr).cuda()
        self.pipeline = pipe
        log("Pixal3D loaded")

    def unload(self) -> None:
        if self.pipeline is None:
            return
        self.pipeline = None
        free_gpu()
        log("Pixal3D unloaded")

    def reset_memory(self) -> None:
        """After a stop or an error: in low-VRAM mode every part should sit in
        normal memory between jobs, but a job cut short can leave one on the
        graphics card."""
        pipe = self.pipeline
        if pipe is None or not pipe.low_vram:
            free_gpu()
            return
        for m in list(pipe.models.values()) + [
            getattr(pipe, a, None)
            for a in ("image_cond_model_ss", "image_cond_model_shape_512", "image_cond_model_shape_1024", "image_cond_model_tex_1024")
        ]:
            try:
                if m is not None:
                    m.cpu()
            except Exception:
                pass
        free_gpu()

    # ---- Generating -------------------------------------------------------

    QUALITY = {
        # pipeline type, steps per stage, default texture size
        "fast": ("1024_cascade", 8, 1024),
        "balanced": ("1024_cascade", 12, 2048),
        "best": ("1536_cascade", 12, 2048),
    }

    def generate(self, job: dict, rep: Reporter) -> str:
        if self.pipeline is None:
            problems = self.problems()
            if problems:
                raise RuntimeError(problems[0])
        import numpy as np
        import torch

        rep.progress("prepare", "Getting the picture ready")
        cutout, how = load_cutout(job["image"])
        rep.log(f"Picture: {how}")
        self.load(rep)
        pipe = self.pipeline
        rep.check()

        pipeline_type, steps, tex_default = self.QUALITY.get(job.get("quality", "balanced"), self.QUALITY["balanced"])
        steps = int(job.get("steps") or steps)
        texture_size = int(job.get("texture_size") or tex_default)
        faces = int(job.get("faces") or 150000)
        seed = int(job.get("seed", 42)) & 0x7FFFFFFF
        max_tokens = int(job.get("max_tokens", 49152))

        image = pipe.preprocess_image(cutout)
        image = _pad_square(image)
        Path(job["out_dir"]).mkdir(parents=True, exist_ok=True)
        try:
            image.save(Path(job["out_dir"]) / "pixal3d_input.png")
        except Exception:
            pass

        # Camera: product pictures are close to a narrow, straight-on lens.
        fov = math.radians(float(job.get("fov_deg", 20.0)))
        camera = camera_from_fov(fov)
        cam = dict(camera_angle_x=camera["camera_angle_x"], distance=camera["distance"], mesh_scale=camera["mesh_scale"])

        ss_params = {"steps": steps, "guidance_strength": 7.5, "guidance_rescale": 0.7, "rescale_t": 5.0}
        shape_params = {"steps": steps, "guidance_strength": 7.5, "guidance_rescale": 0.5, "rescale_t": 3.0}
        tex_params = {"steps": steps, "guidance_strength": 1.0, "guidance_rescale": 0.0, "rescale_t": 3.0}

        torch.manual_seed(seed)
        with torch.no_grad():
            # ---- 1. Rough shape (a 32 x 32 x 32 cloud of cubes) ----
            rep.progress("structure", "Rough shape", 0, steps)
            cond_ss = pipe.get_proj_cond_ss([image], **cam)
            coords = self._sample_structure(pipe, cond_ss, ss_params, steps, rep)
            del cond_ss
            free_gpu()
            rep.voxels(voxel_centres(coords[:, 1:].cpu().numpy(), 32), 1.0 / 32)

            # ---- 2. Shape at 512 ----
            rep.progress("shape", "Shape", 0, steps)
            cond = pipe.get_proj_cond_shape(pipe.image_cond_model_shape_512, [image], coords, **cam)
            with step_hook(pipe.shape_slat_sampler, lambda i: rep.progress("shape", "Shape", i, steps)):
                lr_slat = pipe.sample_shape_slat(cond, pipe.models["shape_slat_flow_model_512"], coords, shape_params)
            del cond
            free_gpu()
            if rep.live:
                rep.progress("shape", "Shape: drawing a preview", steps, steps)
                self._send_mesh_preview(pipe, lr_slat, 512, rep)

            # ---- 3. Fine detail (1024 or 1536) ----
            hr_res = 1536 if pipeline_type == "1536_cascade" else 1024
            if pipe.low_vram:
                pipe.models["shape_slat_decoder"].to(pipe.device)
                pipe.models["shape_slat_decoder"].low_vram = True
            hr_coords = pipe.models["shape_slat_decoder"].upsample(lr_slat, upsample_times=4)
            if pipe.low_vram:
                pipe.models["shape_slat_decoder"].cpu()
                pipe.models["shape_slat_decoder"].low_vram = False
            while True:
                grid = hr_res // 16
                quant = torch.cat([hr_coords[:, :1], ((hr_coords[:, 1:] + 0.5) / 512 * (grid - 1)).round().int()], dim=1)
                hr_unique = quant.unique(dim=0)
                if hr_unique.shape[0] < max_tokens or hr_res == 1024:
                    break
                hr_res -= 128
            del lr_slat, hr_coords, quant
            free_gpu()
            rep.log(f"Detail resolution {hr_res}, {hr_unique.shape[0]} parts")

            from pixal3d.modules.sparse import SparseTensor

            rep.progress("detail", "Fine detail", 0, steps)
            cond = pipe.get_proj_cond_shape(
                pipe.image_cond_model_shape_1024, [image], hr_unique, **cam, grid_resolution_override=hr_res // 16
            )
            flow = pipe.models["shape_slat_flow_model_1024"]
            noise = SparseTensor(feats=torch.randn(hr_unique.shape[0], flow.in_channels).to(pipe.device), coords=hr_unique)
            if pipe.low_vram:
                flow.to(pipe.device)
            with step_hook(pipe.shape_slat_sampler, lambda i: rep.progress("detail", "Fine detail", i, steps)):
                slat = pipe.shape_slat_sampler.sample(
                    flow, noise, **cond, **{**pipe.shape_slat_sampler_params, **shape_params}, verbose=False
                ).samples
            if pipe.low_vram:
                flow.cpu()
            std = torch.tensor(pipe.shape_slat_normalization["std"])[None].to(slat.device)
            mean = torch.tensor(pipe.shape_slat_normalization["mean"])[None].to(slat.device)
            shape_slat = slat * std + mean
            del cond, noise, slat, hr_unique
            free_gpu()

            # ---- 4. Colours ----
            rep.progress("texture", "Colours", 0, steps)
            cond = pipe.get_proj_cond_shape(
                pipe.image_cond_model_tex_1024, [image], shape_slat.coords, **cam, grid_resolution_override=hr_res // 16
            )
            with step_hook(pipe.tex_slat_sampler, lambda i: rep.progress("texture", "Colours", i, steps)):
                tex_slat = pipe.sample_tex_slat(cond, pipe.models["tex_slat_flow_model_1024"], shape_slat, tex_params)
            del cond
            free_gpu()

            # ---- 5. Mesh ----
            rep.progress("decode", "Building the mesh")
            mesh = pipe.decode_latent(shape_slat, tex_slat, hr_res)[0]
            del shape_slat, tex_slat
            free_gpu()
            if rep.live:
                self._send_mesh(mesh.vertices, mesh.faces, rep)

        # ---- 6. Bake the texture and save ----
        rep.progress("bake", "Baking the texture")
        glb = self.postprocess.to_glb(
            vertices=mesh.vertices,
            faces=mesh.faces,
            attr_volume=mesh.attrs,
            coords=mesh.coords,
            attr_layout=pipe.pbr_attr_layout,
            grid_size=hr_res,
            aabb=[[-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]],
            decimation_target=faces,
            texture_size=texture_size,
            remesh=True,
            remesh_band=1,
            remesh_project=0,
            use_tqdm=False,
        )
        rot = np.array([[-1, 0, 0, 0], [0, 0, -1, 0], [0, -1, 0, 0], [0, 0, 0, 1]], dtype=np.float64)
        glb.apply_transform(rot)
        out = Path(job["out_dir"]) / f"garment_{time.strftime('%Y%m%d_%H%M%S')}.glb"
        glb.export(str(out), extension_webp=False)
        del mesh, glb
        free_gpu()
        return str(out)

    def _sample_structure(self, pipe, cond, params, steps, rep):
        """Pixal3D's sparse-structure step, with a cube preview after each step."""
        import torch

        flow = pipe.models["sparse_structure_flow_model"]
        decoder = pipe.models["sparse_structure_decoder"]
        noise = torch.randn(1, flow.in_channels, flow.resolution, flow.resolution, flow.resolution).to(pipe.device)
        if pipe.low_vram:
            flow.to(pipe.device)
            if rep.live:
                decoder.to(pipe.device)

        def on_step(i, out):
            rep.progress("structure", "Rough shape", i, steps)
            if rep.live and i < steps:
                try:
                    occ = decoder(out.pred_x_0) > 0
                    coords = _occupancy_coords(occ, 32)
                    rep.voxels(voxel_centres(coords.cpu().numpy(), 32), 1.0 / 32)
                except Cancelled:
                    raise
                except Exception as exc:  # a preview must never stop the job
                    log(f"Cube preview skipped: {exc}")

        with step_hook(pipe.sparse_structure_sampler, on_step, pass_out=True):
            z = pipe.sparse_structure_sampler.sample(
                flow, noise, **cond, **{**pipe.sparse_structure_sampler_params, **params}, verbose=False
            ).samples
        if pipe.low_vram:
            flow.cpu()
            decoder.to(pipe.device)
        occ = decoder(z) > 0
        if pipe.low_vram:
            decoder.cpu()
        return _occupancy_coords(occ, 32)

    def _send_mesh_preview(self, pipe, slat, resolution, rep):
        try:
            meshes, _ = pipe.decode_shape_slat(slat, resolution)
            self._send_mesh(meshes[0].vertices, meshes[0].faces, rep)
            del meshes
        except Cancelled:
            raise
        except Exception as exc:
            log(f"Mesh preview skipped: {exc}")
        free_gpu()

    def _send_mesh(self, vertices, faces, rep, target: int = 200_000):
        """Send a grey preview mesh, simplified so it's quick to draw."""
        import numpy as np
        import torch

        try:
            v = vertices.float().contiguous()
            f = faces.int().contiguous()
            if f.shape[0] > target:
                cumesh = sys.modules.get("cumesh")
                m = cumesh.CuMesh()
                m.init(v.cuda(), f.cuda())
                m.simplify(target, verbose=False)
                v, f = m.read()
            v = v.detach().float().cpu().numpy()
            f = f.detach().cpu().numpy().astype(np.uint32)
            v = v @ np.array(ROT, dtype=np.float32).T
            rep.mesh(v.reshape(-1), f.reshape(-1))
        except Cancelled:
            raise
        except Exception as exc:
            log(f"Mesh preview skipped: {exc}")
        finally:
            del vertices, faces
            torch.cuda.empty_cache()


# ---- Small helpers ----------------------------------------------------------


class step_hook:
    """Call `fn(step)` after every step of a sampler (and check Stop)."""

    def __init__(self, sampler, fn, pass_out: bool = False):
        self.sampler = sampler
        self.fn = fn
        self.pass_out = pass_out
        self.count = 0

    def __enter__(self):
        original = self.sampler.sample_once

        def wrapped(*args, **kwargs):
            out = original(*args, **kwargs)
            self.count += 1
            if self.pass_out:
                self.fn(self.count, out)
            else:
                self.fn(self.count)
            return out

        self.sampler.sample_once = wrapped
        return self

    def __exit__(self, *exc):
        try:
            del self.sampler.sample_once  # back to the class method
        except AttributeError:
            pass
        return False


def _occupancy_coords(occ, resolution: int):
    """Occupancy grid [1, 1, R, R, R] -> int coords [N, 4] at `resolution`."""
    import torch

    if occ.shape[2] != resolution:
        ratio = occ.shape[2] // resolution
        occ = torch.nn.functional.max_pool3d(occ.float(), ratio, ratio, 0) > 0.5
    return torch.argwhere(occ)[:, [0, 2, 3, 4]].int()


def voxel_centres(xyz, resolution: int):
    """Voxel indices -> centres in the -0.5..0.5 box, turned Y-up."""
    import numpy as np

    p = (np.asarray(xyz, dtype=np.float32) + 0.5) / resolution - 0.5
    return (p @ np.array(ROT, dtype=np.float32).T).reshape(-1)


def camera_from_fov(camera_angle_x: float, mesh_scale: float = 1.0, image_resolution: int = 512) -> dict:
    """Camera distance that fits the whole box in the picture (as Pixal3D does)."""
    focal = 16.0 / math.tan(camera_angle_x / 2.0)
    f_pixels = focal * image_resolution / 32.0
    # grid point (-1, 0, 0) turned by Pixal3D's camera rotation, then scaled.
    xw, yw = -1.0 / mesh_scale / 2, 0.0
    x_ndc = 0 - image_resolution / 2.0
    distance = f_pixels * xw / x_ndc - yw
    return {"camera_angle_x": camera_angle_x, "distance": distance, "mesh_scale": mesh_scale}


def _pad_square(image):
    from PIL import Image

    w, h = image.size
    if w == h:
        return image.convert("RGB")
    s = max(w, h)
    canvas = Image.new("RGB", (s, s), (0, 0, 0))
    canvas.paste(image.convert("RGB"), ((s - w) // 2, (s - h) // 2))
    return canvas


def gpu_info() -> tuple[str, float]:
    try:
        import torch

        if torch.cuda.is_available():
            props = torch.cuda.get_device_properties(0)
            return props.name, round(props.total_memory / 1024**3, 1)
        return "No CUDA graphics card found", 0.0
    except Exception as exc:  # noqa: BLE001
        return f"PyTorch problem: {exc}", 0.0


def free_gpu() -> None:
    import gc

    gc.collect()
    try:
        import torch

        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:
        pass


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------


def main() -> int:
    global _LOG_FILE
    parser = argparse.ArgumentParser(description="ThreadSmith Engine")
    parser.add_argument("--port", type=int, required=True, help="ThreadSmith's local port to connect to")
    parser.add_argument("--comfy-root", default=None, help="ComfyUI install folder (for models and the Pixal3D nodes)")
    parser.add_argument("--log", default=None, help="Log file")
    parser.add_argument("--fake", action="store_true", help="Test mode: pretend to generate")
    args = parser.parse_args()

    if args.log:
        Path(args.log).parent.mkdir(parents=True, exist_ok=True)
        _LOG_FILE = open(args.log, "w", encoding="utf-8")
    fake = args.fake or os.environ.get("THREADSMITH_ENGINE_FAKE") == "1"
    log(f"ThreadSmith Engine {ENGINE_VERSION} (Python {sys.version.split()[0]}, {'test mode' if fake else 'Pixal3D'})")

    conn = Connection(args.port)
    model = FakeModel(args.comfy_root) if fake else Pixal3DModel(args.comfy_root)
    conn.send({"type": "hello", "protocol": PROTOCOL, "version": ENGINE_VERSION, "model": model.name})

    jobs: queue.Queue = queue.Queue()
    cancel = threading.Event()

    def reader():
        try:
            while True:
                header, data = conn.recv()
                kind = header.get("type")
                if kind == "cancel":
                    cancel.set()
                elif kind == "shutdown":
                    jobs.put(None)
                    return
                else:
                    jobs.put(header)
        except Exception as exc:  # ThreadSmith closed: quit
            log(f"Connection closed ({exc}); quitting")
            os._exit(0)

    threading.Thread(target=reader, daemon=True).start()

    while True:
        job = jobs.get()
        if job is None:
            break
        kind = job.get("type")
        try:
            if kind == "status":
                conn.send({"type": "status", **model.status()})
            elif kind == "unload":
                model.unload()
                conn.send({"type": "status", **model.status()})
            elif kind == "generate":
                cancel.clear()
                rep = Reporter(conn, cancel, bool(job.get("live", True)))
                started = time.time()
                log(f"Job: {job.get('image')} ({job.get('quality')})")
                path = model.generate(job, rep)
                seconds = round(time.time() - started, 1)
                log(f"Done in {seconds}s: {path}")
                conn.send({"type": "done", "glb_path": path, "seconds": seconds})
            else:
                conn.send({"type": "error", "message": f"Unknown request {kind!r}"})
        except Cancelled:
            log("Stopped")
            model.reset_memory()
            conn.send({"type": "error", "message": "Stopped.", "cancelled": True})
        except Exception as exc:  # noqa: BLE001
            log(traceback.format_exc())
            message = f"{exc}"
            if type(exc).__name__ in ("OutOfMemoryError",) or "out of memory" in message.lower():
                message = (
                    "The graphics card ran out of memory. Close other programs that use it "
                    "(games, ComfyUI tabs), or try Fast quality."
                )
            model.reset_memory()
            conn.send({"type": "error", "message": message, "detail": traceback.format_exc()[-2000:]})
    log("Shutting down")
    return 0


if __name__ == "__main__":
    sys.exit(main())
