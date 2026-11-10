"""Install Pixal3D for ThreadSmith (run by "Install Pixal3D.bat").

Run it with the Python from your ComfyUI install, for example:
    C:\\ComfyUI-Easy-Install\\ComfyUI-Easy-Install\\python_embeded\\python.exe install_pixal3d.py C:\\ComfyUI-Easy-Install\\ComfyUI-Easy-Install

What it does (and skips whatever is already there):
1. Adds the Pixal3D ComfyUI nodes (Saganaki22/Pixal3D-ComfyUI) to ComfyUI\\custom_nodes.
   ThreadSmith's engine uses their Windows-ready copy of the Pixal3D code.
   You can also use them inside ComfyUI.
2. Adds the Python packages Pixal3D needs that aren't there yet. It never
   replaces torch, transformers or other packages ComfyUI already has.
3. Adds the four CUDA packages (flex_gemm_ap, cumesh_vb, o_voxel_vb_ap, drtk)
   built for your exact Python / PyTorch / CUDA.
4. Signs you in to Hugging Face (needed once: the Pixal3D download asks you
   to accept its terms) and downloads the model (~12 GB) and the DINOv3
   picture reader (~1.2 GB) into ComfyUI\\models\\Pixal3D.
5. Checks that everything loads.
"""

from __future__ import annotations

import importlib
import importlib.util
import io
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import webbrowser
import zipfile
from pathlib import Path

NODES_REPO = "Saganaki22/Pixal3D-ComfyUI"
# The nodes version ThreadSmith was written against.
NODES_COMMIT = "d7e579e6608e74dada99cbf7623d7810e8f6b9ca"
NODES_FOLDER = "Pixal3D-ComfyUI"

MODEL_REPO = "TencentARC/Pixal3D"
MODEL_FOLDER = "TencentARC_Pixal3D"
DINO_REPO = "camenduru/dinov3-vitl16-pretrain-lvd1689m"
DINO_FOLDER = "camenduru_dinov3-vitl16-pretrain-lvd1689m"

# import name -> pip install spec. Only installed when the import fails.
PACKAGES = [
    ("easydict", "easydict"),
    ("einops", "einops"),
    ("timm", "timm"),
    ("plyfile", "plyfile"),
    ("trimesh", "trimesh"),
    ("scipy", "scipy"),
    ("safetensors", "safetensors"),
    ("accelerate", "accelerate"),
    ("diffusers", "diffusers"),
    ("imageio", "imageio"),
    ("natten", "natten==0.21.6"),
    ("utils3d", "https://github.com/LDYang694/Storages/releases/download/20260430/utils3d-0.0.2-py3-none-any.whl"),
]

WHEEL_BASE = "https://github.com/PozzettiAndrea/cuda-wheels/releases/download"
# (python tag, torch tag, cuda tag) -> wheels. Same list as the Pixal3D nodes' installer.
CUDA_WHEELS = {}
for _torch, _cuda in (("torch2.8", "cu128"), ("torch2.9", "cu128"), ("torch2.9", "cu130"), ("torch2.10", "cu130")):
    CUDA_WHEELS[("cp312", _torch, _cuda)] = [
        ("flex_gemm_ap", f"{WHEEL_BASE}/flex_gemm_ap-latest/flex_gemm_ap-1.0.0%2B{_cuda}{_torch}-cp312-cp312-win_amd64.whl"),
        ("cumesh_vb", f"{WHEEL_BASE}/cumesh_vb-latest/cumesh_vb-1.0%2B{_cuda}{_torch}-cp312-cp312-win_amd64.whl"),
        ("o_voxel_vb_ap", f"{WHEEL_BASE}/o_voxel_vb_ap-latest/o_voxel_vb_ap-0.0.1%2B{_cuda}{_torch}-cp312-cp312-win_amd64.whl"),
        ("drtk", f"{WHEEL_BASE}/drtk-latest/drtk-0.1.0%2B{_cuda}{_torch}-cp312-cp312-win_amd64.whl"),
    ]


def say(text: str = "") -> None:
    print(text, flush=True)


def step(n: int, text: str) -> None:
    say()
    say(f"=== {n}. {text} ===")


def has(module: str) -> bool:
    try:
        return importlib.util.find_spec(module) is not None
    except Exception:
        return False


def pip(*args: str) -> bool:
    cmd = [sys.executable, "-m", "pip", "install", "--disable-pip-version-check", *args]
    say("  > pip install " + " ".join(args))
    return subprocess.call(cmd) == 0


def comfy_dirs(root: Path) -> tuple[Path, Path]:
    for d in (root / "ComfyUI", root):
        if (d / "main.py").is_file():
            return d, d / "models"
    raise SystemExit(f"No ComfyUI\\main.py in {root}. Pass your ComfyUI install folder.")


# ---- 1. Nodes ----------------------------------------------------------------


def install_nodes(comfy: Path) -> Path:
    target = comfy / "custom_nodes" / NODES_FOLDER
    if (target / "pixal3d" / "pipelines").is_dir() and (target / "pixal3d_comfy").is_dir():
        say(f"  Already there: {target}")
        return target
    git = shutil.which("git")
    if git:
        say(f"  Cloning {NODES_REPO}...")
        if subprocess.call([git, "clone", f"https://github.com/{NODES_REPO}.git", str(target)]) == 0:
            subprocess.call([git, "-C", str(target), "checkout", "-q", NODES_COMMIT])
            return target
        say("  git clone failed; downloading a zip instead.")
    url = f"https://github.com/{NODES_REPO}/archive/{NODES_COMMIT}.zip"
    say(f"  Downloading {url}")
    data = urllib.request.urlopen(url, timeout=120).read()
    with tempfile.TemporaryDirectory() as tmp:
        zipfile.ZipFile(io.BytesIO(data)).extractall(tmp)
        inner = next(Path(tmp).iterdir())
        shutil.move(str(inner), str(target))
    return target


# ---- 2/3. Packages ---------------------------------------------------------------


def install_packages() -> list[str]:
    problems = []
    for module, spec in PACKAGES:
        if has(module):
            continue
        no_deps = spec.startswith("http") or module == "natten"
        if not pip(*(["--no-deps"] if no_deps else []), spec):
            problems.append(f"Couldn't install {spec}")
    return problems


def install_cuda_wheels() -> list[str]:
    import torch

    py = f"cp{sys.version_info.major}{sys.version_info.minor}"
    t = torch.__version__.split("+")[0].split(".")
    torch_tag = f"torch{t[0]}.{t[1]}"
    cuda_tag = "cu" + "".join((torch.version.cuda or "").split(".")[:2])
    say(f"  Python {py}, PyTorch {torch.__version__}, CUDA {torch.version.cuda}")
    if torch.cuda.is_available():
        say(f"  Graphics card: {torch.cuda.get_device_name(0)}")
    wheels = CUDA_WHEELS.get((py, torch_tag, cuda_tag))
    missing = [(m, u) for m, u in (wheels or []) if not has(m)]
    if wheels is None:
        need = [m for m in ("flex_gemm_ap", "cumesh_vb", "o_voxel_vb_ap", "drtk") if not has(m)]
        if need:
            return [
                f"No ready-made CUDA packages are known for {py} + {torch_tag} + {cuda_tag}. "
                f"Missing: {', '.join(need)}. See https://pozzettiandrea.github.io/cuda-wheels/ for matching ones."
            ]
        return []
    if not missing:
        say("  Already there.")
        return []
    # --no-deps so pip never swaps out the working PyTorch.
    if not pip("--no-deps", *[u for _, u in missing]):
        return ["Couldn't install the CUDA packages (see the messages above)."]
    return []


# ---- 4. Models -------------------------------------------------------------------


def hf_sign_in(repo: str) -> None:
    from huggingface_hub import HfApi, get_token, login

    page = f"https://huggingface.co/{repo}"
    while True:
        token = get_token()
        if token:
            try:
                HfApi().auth_check(repo, token=token)
                say("  Hugging Face: signed in and allowed.")
                return
            except Exception as exc:  # noqa: BLE001
                name = type(exc).__name__
                if "Gated" in name or "403" in str(exc):
                    say(f"  Your account hasn't accepted Pixal3D's terms yet.")
                    say(f"  Opening {page} - sign in, click \"Agree and access repository\".")
                    webbrowser.open(page)
                    input("  Press Enter here once you've clicked Agree... ")
                    continue
                if "401" in str(exc) or "Unauthorized" in name or "Authentication" in name:
                    say("  That token didn't work. Let's set a new one.")
                else:
                    say(f"  Couldn't check access ({name}: {exc}). Trying the download anyway.")
                    return
        say()
        say("  Pixal3D's download needs a free Hugging Face account (once).")
        say(f"  1. Opening {page} - sign in (or sign up) and click \"Agree and access repository\".")
        say("  2. Opening https://huggingface.co/settings/tokens - make a token (type: Read) and copy it.")
        webbrowser.open(page)
        webbrowser.open("https://huggingface.co/settings/tokens")
        token = input("  Paste the token here and press Enter: ").strip()
        if not token:
            raise SystemExit("No token given.")
        login(token=token, add_to_git_credential=False)


def download(repo: str, dest: Path, ignore: list[str]) -> None:
    from huggingface_hub import snapshot_download

    say(f"  {repo} -> {dest}")
    snapshot_download(repo_id=repo, local_dir=str(dest), ignore_patterns=ignore, max_workers=4)


def missing_model_files(model_dir: Path) -> list[str]:
    import json

    p = model_dir / "pipeline.json"
    if not p.is_file():
        return ["pipeline.json"]
    data = json.loads(p.read_text(encoding="utf-8"))
    out = []
    for rel in data.get("args", {}).get("models", {}).values():
        for suffix in (".json", ".safetensors"):
            if not (model_dir / f"{rel}{suffix}").is_file():
                out.append(f"{rel}{suffix}")
    return out


# ---- 5. Check ------------------------------------------------------------------------


def check(root: Path) -> list[str]:
    engine_file = Path(__file__).resolve().parent.parent / "engine" / "threadsmith_engine.py"
    spec = importlib.util.spec_from_file_location("threadsmith_engine", engine_file)
    engine = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(engine)
    model = engine.Pixal3DModel(str(root))
    problems = model.problems()
    if problems:
        return problems
    try:
        model._prepare_imports()
        importlib.import_module("pixal3d.pipelines")
        importlib.import_module("pixal3d.trainers.flow_matching.mixins.image_conditioned_proj")
    except Exception as exc:  # noqa: BLE001
        import traceback

        traceback.print_exc()
        return [f"Pixal3D's code didn't load: {type(exc).__name__}: {exc}"]
    try:
        import natten

        if not getattr(natten, "HAS_LIBNATTEN", False):
            say("  Note: NATTEN has no CUDA kernels here, so Pixal3D's NAF detail step uses its fallback.")
            say("        That's normal on Windows and only costs a little fine detail.")
    except Exception:
        pass
    return []


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(sys.executable).resolve().parent.parent
    comfy, models = comfy_dirs(root)
    say(f"ComfyUI: {comfy}")
    say(f"Python:  {sys.executable}")
    problems: list[str] = []

    step(1, "Pixal3D nodes")
    install_nodes(comfy)

    step(2, "Python packages")
    problems += install_packages()

    step(3, "CUDA packages")
    problems += install_cuda_wheels()

    step(4, "Models")
    model_dir = models / "Pixal3D" / MODEL_FOLDER
    if missing_model_files(model_dir):
        hf_sign_in(MODEL_REPO)
        # The multi-view weights aren't used yet; skipping them saves ~10 GB.
        download(MODEL_REPO, model_dir, ["*_mv*", "pipeline_mv.json", "assets/*", "*.md", ".gitattributes"])
        left = missing_model_files(model_dir)
        if left:
            problems.append("Pixal3D download is missing: " + ", ".join(left[:4]))
    else:
        say("  Pixal3D model: already there.")
    dino_dir = models / "Pixal3D" / DINO_FOLDER
    if not (dino_dir / "config.json").is_file() or not any(dino_dir.glob("*.safetensors")):
        download(DINO_REPO, dino_dir, ["*.md", ".gitattributes"])
    else:
        say("  DINOv3: already there.")

    step(5, "Check")
    problems += check(root)

    say()
    if problems:
        say("Not finished:")
        for p in problems:
            say(f"  - {p}")
        return 1
    say("Pixal3D is ready. In ThreadSmith: Generate > Settings > Check engine.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(1)
