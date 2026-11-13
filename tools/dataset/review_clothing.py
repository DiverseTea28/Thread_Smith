"""Check and review the clothing models collected by collect_clothing.py.

Run it with "Review clothing.bat" (in the Training data folder), or:
    python review_clothing.py "C:\\Users\\you\\Documents\\ThreadSmith\\Training data"

What it does
1. Shape checks for every collected model (saved in _state\\checks.jsonl, so each
   model is only checked once). It reads the mesh and looks for:
   - a name that suggests it isn't clothing ("coat of arms", "puffer fish"...)
   - a closed, solid shape with no openings (a garment needs sleeve/neck/waist holes)
   - several big separate pieces (maybe a full outfit or a set)
   - duplicates (same file, or same shape and size)
   - very few triangles, no texture, flat like a card, rigged (may hold a body)
2. Pictures: the review page (http://127.0.0.1:8765) makes 4 still pictures of every
   model (front, 3/4, side, back) in your browser and saves them in _state\\thumbs.
   The grid shows these pictures, so a page of 48 models loads instantly.
   Leave the page open in front while it makes them (a hidden tab pauses it).
3. AI check (CLIP): the tool looks at the 4 pictures and guesses
   - is it clothing, a body/mannequin/character, or something else (hanger, golf tee...)
   - which garment type it is (Shoes, Head, Shirts...)
   It runs on your graphics card with ComfyUI's Python. The first run downloads the
   CLIP model (about 1.7 GB, openai/clip-vit-large-patch14) from Hugging Face.
   Results are saved in _state\\ai.jsonl.
4. Learning from you: once you have marked at least 25 Keep and 25 Reject yourself,
   the tool learns what you keep and gives each model a "keep chance".
5. Auto-sort into three piles (only models you haven't marked yourself):
   - Auto-rejected: Roblox, "low poly" in the name, under --min-tris triangles, an exact
     copy of another model, AI sure it isn't clothing, or very like the ones you rejected.
   - Auto-kept: AI sure it is clothing of the right type, no problems found, and (once it
     has learned) a high keep chance.
   - Needs you: everything else. This is the pile you review.
   Auto choices are marked "auto" and are never forced: change any of them on the page.
   They are worked out again every time (your own choices are never touched).
6. "Apply decisions" (button on the page, or --apply) moves files:
   - Rejected  -> _rejected\\<type>\\   (nothing is deleted; un-reject and apply again to bring it back)
   - Edit      -> _edit\\<type>\\       (good garment with extra stuff, like a mannequin: open it in
                                       Blender, delete the extra parts, export the GLB over the same
                                       file, then mark it Keep and apply again)
   - New type  -> that type's folder
   - Kept from _check -> its type's folder
   and rewrites CREDITS.md / credits.csv without the rejected models.

Needs numpy (ComfyUI's Python has it). The AI check also needs torch, transformers
and Pillow (ComfyUI's Python has them too); without them the rest still works.
The page loads Google's free <model-viewer> from the internet to show the 3D models.
"""

from __future__ import annotations

import argparse
import base64
import concurrent.futures as cf
import csv
import hashlib
import json
import math
import os
import re
import shutil
import struct
import sys
import threading
import time
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

try:
    import numpy as np
except ImportError:  # pragma: no cover
    print("This tool needs numpy. Run it with ComfyUI's Python (Review clothing.bat does that).")
    sys.exit(1)

CHECK_VERSION = 1  # bump to re-check every model after changing the checks
AI_VERSION = 1     # bump to re-run the AI check after changing the labels below

TYPES = ["Accessory", "Backpacks", "Belts", "Dresses", "Full Body", "Gloves", "Head", "Jackets", "Pants",
         "Shirts", "Shoes", "Skirts", "Vests"]

# Garment types that should have openings (sleeves, neck, waist, legs...).
NEEDS_OPENINGS = {"Shirts", "Jackets", "Vests", "Dresses", "Pants", "Skirts", "Full Body", "Gloves"}

# Name words that suggest "not a garment" or "more than one garment".
NOT_CLOTHING = [
    "coat of arms", "coat of arm", "3d coat", "3dcoat", "coat hook", "coat stand", "coat rail", "coat rack",
    "coat racks", "hanger", "hangers", "puffer fish", "pufferfish", "fish", "chevy", "chevrolet", "van", "car",
    "mushroom", "mushrooms", "spongebob", "sponge bob", "tee box", "tee stone", "hole", "golf tee", "church",
    "cemetery", "cemetary", "monument", "statue", "cnc", "skull", "bust",
    "character", "girl", "boy", "man", "woman", "lady", "person", "human", "body", "mannequin", "doll", "toy",
    "figure", "robot", "creature", "animal", "kennedy", "leon", "krauser",
    "fatman", "pirate torso", "stakan", "glass", "watchtower", "asteroid", "conveyor",
    "pool table", "machine", "printable", "scene", "room", "shop", "store",
]
OUTFIT_WORDS = ["and", "outfit", "set", "top and", "2 piece", "two piece", "pieces", "bundle", "pack"]

# ---------------------------------------------------------------------------
# What the AI (CLIP) chooses between. It picks whichever words fit the pictures
# best; we then add up "clothing", "body" and "other". Add words here if it keeps
# missing something (then bump AI_VERSION so every model is checked again).
# ---------------------------------------------------------------------------
CLOTHING_LABELS = {
    "Shirts": ["a t-shirt", "a shirt", "a blouse", "a sweater", "a hoodie", "a tank top", "a crop top"],
    "Jackets": ["a jacket", "a coat", "a leather jacket", "a puffer jacket", "a blazer"],
    "Vests": ["a vest", "a waistcoat", "a tactical vest"],
    "Pants": ["a pair of pants", "a pair of jeans", "a pair of shorts", "a pair of trousers", "leggings"],
    "Skirts": ["a skirt"],
    "Dresses": ["a dress", "a gown"],
    "Full Body": ["a jumpsuit", "overalls", "a full body suit", "a onesie", "a space suit"],
    "Gloves": ["a glove", "a pair of gloves", "a gauntlet"],
    "Shoes": ["a shoe", "a sneaker", "a boot", "a high heel shoe", "a sandal", "a slipper"],
    "Head": ["a hat", "a baseball cap", "a beanie", "a helmet", "a hood", "a face mask", "a crown", "a headband"],
    "Belts": ["a belt"],
    "Backpacks": ["a backpack", "a bag", "a purse", "a handbag"],
    "Accessory": ["a pair of sunglasses", "a pair of glasses", "a wristwatch", "a necklace", "an earring", "a ring",
                  "a bracelet", "a scarf", "a necktie"],
}
BODY_LABELS = ["a person wearing clothes", "a 3D character", "a mannequin", "a doll", "a naked human body",
               "a statue of a person", "a bust of a head", "a mannequin head"]
OTHER_LABELS = ["a clothes hanger", "a golf tee", "a coat rack", "a coat of arms", "a shield", "a fish", "a car",
                "a truck", "a building", "a room", "a piece of furniture", "a chair", "a lamp", "a sword", "a gun",
                "a tool", "a toy", "a plant", "a mushroom", "a tree", "a rock", "a bottle", "a cup",
                "a kitchen appliance", "an animal", "a robot", "a machine", "a box", "a sign", "a sculpture",
                "food", "a planet", "a vase", "a pillow", "a curtain", "a flag", "a blanket"]
PROMPT_TEMPLATES = ["a 3D model of {}.", "a 3D render of {}, on a grey background.", "a render of {}."]


def all_labels() -> tuple[list[str], list[str], list[str]]:
    """(label, kind, type) lists. kind is cloth / body / other."""
    labels, kinds, types = [], [], []
    for t, ls in CLOTHING_LABELS.items():
        for lab in ls:
            labels.append(lab), kinds.append("cloth"), types.append(t)
    for lab in BODY_LABELS:
        labels.append(lab), kinds.append("body"), types.append("")
    for lab in OTHER_LABELS:
        labels.append(lab), kinds.append("other"), types.append("")
    return labels, kinds, types


def labels_key() -> str:
    labels, kinds, _ = all_labels()
    return hashlib.sha1(json.dumps([labels, kinds, PROMPT_TEMPLATES]).encode()).hexdigest()[:10]


# "low poly", "lowpoly", "low-poly", "low_poly", and the common typo "low ploy".
LOW_POLY = re.compile(r"(?<![a-z])low[\s_-]*(?:poly|ploy)(?!ester)", re.I)


def words(text: str) -> str:
    return " " + re.sub(r"[^a-z0-9]+", " ", (text or "").lower()).strip() + " "


def has_word(haystack: str, word: str) -> bool:
    return f" {words(word).strip()} " in haystack


def safe_name(uid: str) -> str:
    return re.sub(r"[^A-Za-z0-9_-]", "_", uid)[:120]


def pct(x: float | None) -> str:
    return "?" if x is None else f"{round(100 * x)}%"


# ---------------------------------------------------------------------------
# Reading a GLB mesh (positions + triangles, in world space)
# ---------------------------------------------------------------------------

COMP = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


def read_glb(path: Path) -> tuple[dict, bytes]:
    data = path.read_bytes()
    magic, _v, _len = struct.unpack_from("<III", data, 0)
    if magic != 0x46546C67:
        raise ValueError("not a GLB")
    off, js, binc = 12, None, b""
    while off + 8 <= len(data):
        clen, ctype = struct.unpack_from("<II", data, off)
        chunk = data[off + 8: off + 8 + clen]
        if ctype == 0x4E4F534A:
            js = json.loads(chunk)
        elif ctype == 0x004E4942:
            binc = chunk
        off += 8 + clen
    if js is None:
        raise ValueError("no JSON chunk")
    return js, binc


def accessor(js: dict, binc: bytes, i: int) -> np.ndarray:
    a = js["accessors"][i]
    n, dt = NCOMP[a["type"]], np.dtype(COMP[a["componentType"]])
    count = a["count"]
    if "bufferView" not in a:
        return np.zeros((count, n), dt)
    bv = js["bufferViews"][a["bufferView"]]
    if bv.get("buffer", 0) != 0:
        raise ValueError("external buffer")
    start = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride") or dt.itemsize * n
    arr = np.ndarray((count, n), dt, buffer=binc, offset=start, strides=(stride, dt.itemsize))
    return np.array(arr)


def node_matrix(node: dict) -> np.ndarray:
    if "matrix" in node:
        return np.array(node["matrix"], dtype=np.float64).reshape(4, 4).T
    t = np.array(node.get("translation", [0, 0, 0]), dtype=np.float64)
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    s = np.array(node.get("scale", [1, 1, 1]), dtype=np.float64)
    r = np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ])
    m = np.eye(4)
    m[:3, :3] = r * s
    m[:3, 3] = t
    return m


def mesh_world(js: dict, binc: bytes) -> tuple[np.ndarray, np.ndarray, bool]:
    """All triangles of the default scene in world space: (positions, faces, compressed?)."""
    nodes = js.get("nodes", [])
    scenes = js.get("scenes", [])
    if scenes:
        roots = scenes[js.get("scene", 0)].get("nodes", [])
    else:
        children = {c for n in nodes for c in n.get("children", [])}
        roots = [i for i in range(len(nodes)) if i not in children]
    pos_all, faces_all, base, compressed = [], [], 0, False
    stack = [(r, np.eye(4)) for r in roots]
    seen = 0
    while stack:
        ni, parent = stack.pop()
        seen += 1
        if seen > 100000:
            break
        node = nodes[ni]
        m = parent @ node_matrix(node)
        for c in node.get("children", []):
            stack.append((c, m))
        if "mesh" not in node:
            continue
        for prim in js["meshes"][node["mesh"]].get("primitives", []):
            ext = prim.get("extensions", {})
            if "KHR_draco_mesh_compression" in ext:
                compressed = True
                continue
            mode = prim.get("mode", 4)
            if mode not in (4, 5, 6) or "POSITION" not in prim.get("attributes", {}):
                continue
            p = accessor(js, binc, prim["attributes"]["POSITION"]).astype(np.float64)[:, :3]
            if "indices" in prim:
                idx = accessor(js, binc, prim["indices"]).astype(np.int64).ravel()
            else:
                idx = np.arange(len(p), dtype=np.int64)
            if mode == 4:
                f = idx[: len(idx) // 3 * 3].reshape(-1, 3)
            elif mode == 5:  # strip
                k = np.arange(len(idx) - 2)
                f = np.stack([idx[k], idx[k + 1 + (k % 2)], idx[k + 2 - (k % 2)]], 1)
            else:  # fan
                k = np.arange(1, len(idx) - 1)
                f = np.stack([np.full_like(k, idx[0]), idx[k], idx[k + 1]], 1)
            p = (np.c_[p, np.ones(len(p))] @ m.T)[:, :3]
            pos_all.append(p)
            faces_all.append(f + base)
            base += len(p)
    if any("EXT_meshopt_compression" in bv.get("extensions", {}) for bv in js.get("bufferViews", [])):
        compressed = True
    if not pos_all:
        return np.zeros((0, 3)), np.zeros((0, 3), np.int64), compressed
    return np.concatenate(pos_all), np.concatenate(faces_all), compressed


def components(n: int, edges: np.ndarray) -> np.ndarray:
    """Connected-component label per vertex (union-find in numpy)."""
    parent = np.arange(n)
    if len(edges) == 0:
        return parent
    a, b = edges[:, 0], edges[:, 1]
    for _ in range(200):
        pa, pb = parent[a], parent[b]
        lo = np.minimum(pa, pb)
        changed = (pa != lo) | (pb != lo)
        if not changed.any():
            break
        np.minimum.at(parent, pa, lo)
        np.minimum.at(parent, pb, lo)
        # pointer jumping
        for _j in range(8):
            nxt = parent[parent]
            if np.array_equal(nxt, parent):
                break
            parent = nxt
    while True:
        nxt = parent[parent]
        if np.array_equal(nxt, parent):
            return parent
        parent = nxt


def shape_check(path: Path) -> dict:
    """Mesh numbers used for the hints."""
    js, binc = read_glb(path)
    pos, faces, compressed = mesh_world(js, binc)
    out = {"compressed": compressed, "textures": len(js.get("images", [])),
           "skins": len(js.get("skins", [])), "animations": len(js.get("animations", []))}
    if len(faces) == 0:
        out["tris"] = 0
        return out
    lo, hi = pos.min(0), pos.max(0)
    size = hi - lo
    diag = float(np.linalg.norm(size)) or 1.0
    out["size"] = [round(float(x), 4) for x in size]
    # Weld: vertices split at UV seams / material borders are the same point.
    q = np.round((pos - lo) / (diag * 1e-5)).astype(np.int64)
    _, inv = np.unique(q, axis=0, return_inverse=True)
    inv = inv.ravel()
    f = inv[faces]
    good = (f[:, 0] != f[:, 1]) & (f[:, 1] != f[:, 2]) & (f[:, 0] != f[:, 2])
    f = f[good]
    nv = int(inv.max()) + 1
    out["tris"] = int(len(f))
    e = np.sort(np.concatenate([f[:, [0, 1]], f[:, [1, 2]], f[:, [2, 0]]]), axis=1)
    ue, cnt = np.unique(e, axis=0, return_counts=True)
    boundary = ue[cnt == 1]
    out["open_edges"] = int(len(boundary))
    out["bad_edges"] = int((cnt > 2).sum())
    # Openings = loops of open edges.
    if len(boundary):
        bl = components(nv, boundary)
        bverts = np.unique(boundary)
        loops, li = np.unique(bl[bverts], return_inverse=True)
        out["openings"] = int(len(loops))
        # Loop size vs model size, so tiny holes (mesh errors) can be told apart from sleeves.
        wpos = np.zeros((nv, 3))
        wpos[inv] = pos
        bp = wpos[bverts]
        mn = np.full((len(loops), 3), np.inf)
        mx = np.full((len(loops), 3), -np.inf)
        np.minimum.at(mn, li.ravel(), bp)
        np.maximum.at(mx, li.ravel(), bp)
        out["big_openings"] = int(((mx - mn).max(1) > 0.08 * diag).sum())
    else:
        out["openings"] = 0
        out["big_openings"] = 0
    # Separate pieces, and how many are big (>10% of the triangles).
    lab = components(nv, ue)
    used = np.unique(f)
    face_lab = lab[f[:, 0]]
    _, piece_tris = np.unique(face_lab, return_counts=True)
    out["pieces"] = int(len(piece_tris))
    out["big_pieces"] = int((piece_tris > 0.10 * len(f)).sum())
    # Euler number -> handles. A thick garment (closed, with tunnels) has handles instead of open edges.
    v_used = len(used)
    chi = v_used - len(ue) + len(f)
    genus = (2 * out["pieces"] - chi - out["openings"]) / 2
    out["handles"] = int(max(round(genus), 0))
    return out


def sha1(path: Path) -> str:
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def check_one(path_str: str) -> dict:
    path = Path(path_str)
    out = {"sha1": sha1(path)}
    try:
        out.update(shape_check(path))
    except Exception as exc:  # noqa: BLE001
        out["error"] = f"{type(exc).__name__}: {exc}"[:200]
    return out


# ---------------------------------------------------------------------------
# Learning from your own Keep / Reject choices (small logistic regression)
# ---------------------------------------------------------------------------


def fit_logreg(x: np.ndarray, y: np.ndarray, lam: float, its: int = 30) -> np.ndarray:
    """Weights (last one is the bias) for P(keep) = sigmoid(x @ w)."""
    n, d = x.shape
    xb = np.c_[x, np.ones(n)]
    w = np.zeros(d + 1)
    reg = lam * np.eye(d + 1)
    reg[-1, -1] = 1e-6
    for _ in range(its):
        p = 1 / (1 + np.exp(-np.clip(xb @ w, -30, 30)))
        g = xb.T @ (p - y) + reg @ w
        h = (xb * (p * (1 - p))[:, None]).T @ xb + reg
        step = np.linalg.solve(h, g)
        w -= step
        if np.abs(step).max() < 1e-7:
            break
    return w


def predict_logreg(x: np.ndarray, w: np.ndarray) -> np.ndarray:
    return 1 / (1 + np.exp(-np.clip(np.c_[x, np.ones(len(x))] @ w, -30, 30)))


def learn_keep_chance(feats: dict[str, np.ndarray], labels: dict[str, int]) -> tuple[dict[str, float], float]:
    """Train on your decisions, then guess a keep chance for every model.
    Returns (uid -> chance, accuracy on models it didn't train on)."""
    uids = list(feats)
    x = np.stack([feats[u] for u in uids])
    mu, sd = x.mean(0), x.std(0) + 1e-6
    x = (x - mu) / sd
    tr = [i for i, u in enumerate(uids) if u in labels]
    xt, yt = x[tr], np.array([labels[uids[i]] for i in tr], float)
    rng = np.random.default_rng(0)
    fold = rng.permutation(len(tr)) % 5
    best = (1e9, 10.0, 0.0)
    for lam in (3.0, 30.0, 300.0):
        loss, right = 0.0, 0
        for k in range(5):
            a, b = fold != k, fold == k
            p = predict_logreg(xt[b], fit_logreg(xt[a], yt[a], lam))
            loss += -np.sum(yt[b] * np.log(p + 1e-9) + (1 - yt[b]) * np.log(1 - p + 1e-9))
            right += int(((p > 0.5) == (yt[b] > 0.5)).sum())
        if loss < best[0]:
            best = (loss, lam, right / len(tr))
    w = fit_logreg(xt, yt, best[1])
    p = predict_logreg(x, w)
    return {u: float(p[i]) for i, u in enumerate(uids)}, best[2]


# ---------------------------------------------------------------------------
# Data: models, checks, pictures, AI results, decisions
# ---------------------------------------------------------------------------


class Dataset:
    def __init__(self, root: Path, ai_model: str):
        self.root = root
        self.state = root / "_state"
        self.thumbs = self.state / "thumbs"
        self.thumbs.mkdir(parents=True, exist_ok=True)
        self.ai_model = ai_model
        self.lock = threading.RLock()
        self.file_lock = threading.Lock()
        self.records: dict[str, dict] = {}
        p = self.state / "done.jsonl"
        if not p.is_file():
            raise SystemExit(f"No {p}: run Collect clothing first.")
        for line in p.read_text(encoding="utf-8").splitlines():
            try:
                r = json.loads(line)
                self.records[r["uid"]] = r
            except Exception:  # noqa: BLE001
                pass
        self.checks: dict[str, dict] = {}
        for c in self._jsonl("checks.jsonl"):
            if c.get("v") == CHECK_VERSION:
                self.checks[c["uid"]] = c
        self.ai: dict[str, dict] = {}
        key = labels_key()
        for a in self._jsonl("ai.jsonl"):
            if a.get("v") == AI_VERSION and a.get("model") == ai_model and a.get("labels") == key:
                self.ai[a["uid"]] = a
        self.thumb_failed: dict[str, dict] = {f["uid"]: f for f in self._jsonl("thumb_failed.jsonl")}
        self.decisions: dict[str, dict] = {}
        rp = root / "review.csv"
        if rp.is_file():
            with open(rp, newline="", encoding="utf-8") as f:
                for row in csv.DictReader(f):
                    self.decisions[row["uid"]] = row
        self.keep_chance: dict[str, float] = {}
        self.learn_info = "not learned yet"
        self.learn_ok = False
        self.sort_info = ""
        self.sorts = 0          # goes up each time the piles are worked out again (the page watches it)
        self.ai_worker = None   # AIChecker, set in main()

    def _jsonl(self, name: str) -> list[dict]:
        p = self.state / name
        out = []
        if p.is_file():
            for line in p.read_text(encoding="utf-8").splitlines():
                try:
                    out.append(json.loads(line))
                except Exception:  # noqa: BLE001
                    pass
        return out

    def append_jsonl(self, name: str, rec: dict) -> None:
        with self.file_lock, open(self.state / name, "a", encoding="utf-8") as f:
            f.write(json.dumps(rec) + "\n")

    def path_of(self, r: dict) -> Path:
        return self.root / r["file"].replace("\\", "/")

    def thumb_path(self, uid: str) -> Path:
        return self.thumbs / f"{safe_name(uid)}.jpg"

    def has_thumb(self, uid: str) -> bool:
        """A picture exists and is newer than the model file."""
        t = self.thumb_path(uid)
        if not t.is_file():
            return False
        m = self.path_of(self.records[uid])
        return not m.is_file() or t.stat().st_mtime >= m.stat().st_mtime - 1

    # ---- auto-checks ----
    def run_checks(self, workers: int) -> None:
        todo = []
        for uid, r in self.records.items():
            p = self.path_of(r)
            if not p.is_file():
                continue
            c = self.checks.get(uid)
            if c and c.get("bytes") == p.stat().st_size:
                continue
            todo.append((uid, p))
        if not todo:
            print(f"Shape checks: all {len(self.checks)} models already checked.")
            return
        total_mb = sum(p.stat().st_size for _, p in todo) / 1e6
        print(f"Shape checks: checking {len(todo)} models ({total_mb / 1000:.1f} GB). "
              f"This is done once; you can stop and start again.")
        t0 = time.time()
        with open(self.state / "checks.jsonl", "a", encoding="utf-8") as out, \
                cf.ProcessPoolExecutor(max_workers=workers) as pool:
            futs = {pool.submit(check_one, str(p)): (uid, p) for uid, p in todo}
            for n, fut in enumerate(cf.as_completed(futs), 1):
                uid, p = futs[fut]
                try:
                    c = fut.result()
                except Exception as exc:  # noqa: BLE001 (e.g. out of memory on a huge file)
                    c = {"error": f"{type(exc).__name__}: {exc}"[:200]}
                c.update({"uid": uid, "v": CHECK_VERSION, "bytes": p.stat().st_size})
                self.checks[uid] = c
                out.write(json.dumps(c) + "\n")
                out.flush()
                if n % 25 == 0 or n == len(todo):
                    el = time.time() - t0
                    left = el / n * (len(todo) - n)
                    print(f"  {n}/{len(todo)} checked ({el / 60:.0f} min so far, about {left / 60:.0f} min left)")

    # ---- pictures ----
    def thumbs_todo(self) -> list[dict]:
        """Models that still need pictures: not-reviewed first, then the ones you decided
        (the AI learns from those), then the auto-rejected ones."""
        out = []
        for uid, r in self.records.items():
            p = self.path_of(r)
            if not p.is_file() or self.has_thumb(uid):
                continue
            size = p.stat().st_size
            if self.thumb_failed.get(uid, {}).get("bytes") == size:
                continue
            d = self.decisions.get(uid, {})
            order = 2 if d.get("note", "").startswith("auto") else (1 if d.get("decision") else 0)
            out.append({"uid": uid, "file": r["file"].replace("\\", "/"), "mb": round(size / 1e6, 1),
                        "order": order})
        out.sort(key=lambda x: (x["order"], x["file"]))
        return out

    def save_thumb(self, uid: str, data: bytes) -> None:
        if uid not in self.records:
            raise ValueError("unknown model")
        if not data.startswith(b"\xff\xd8"):
            raise ValueError("not a JPEG")
        t = self.thumb_path(uid)
        tmp = t.with_suffix(".tmp")
        tmp.write_bytes(data)
        os.replace(tmp, t)
        self.thumb_failed.pop(uid, None)
        if self.ai_worker:
            self.ai_worker.add(uid)

    def thumb_fail(self, uid: str, why: str) -> None:
        if uid not in self.records:
            raise ValueError("unknown model")
        p = self.path_of(self.records[uid])
        rec = {"uid": uid, "bytes": p.stat().st_size if p.is_file() else 0, "why": why[:200]}
        self.thumb_failed[uid] = rec
        self.append_jsonl("thumb_failed.jsonl", rec)

    def ai_todo(self) -> list[str]:
        out = []
        for uid in self.records:
            if not self.has_thumb(uid):
                continue
            a = self.ai.get(uid)
            if a and abs(a.get("thumb_mtime", 0) - self.thumb_path(uid).stat().st_mtime) < 1:
                continue
            out.append(uid)
        return out

    # ---- hints ----
    def hints(self) -> dict[str, tuple[list[str], bool]]:
        """uid -> (problem lines, any serious problem?). Serious problems stop an auto-keep."""
        by_hash: dict[str, list[str]] = {}
        by_shape: dict[tuple, list[str]] = {}
        for uid, c in self.checks.items():
            if uid not in self.records:
                continue
            by_hash.setdefault(c.get("sha1", uid), []).append(uid)
            if c.get("tris") and c.get("size"):
                key = (c["tris"], tuple(round(x, 2) for x in sorted(c["size"])))
                by_shape.setdefault(key, []).append(uid)
        out = {}
        for uid, r in self.records.items():
            c = self.checks.get(uid, {})
            h, serious = [], False
            n = words(r.get("name", ""))
            bad = [w for w in NOT_CLOTHING if has_word(n, w)]
            if bad:
                h.append("name suggests not clothing: " + ", ".join(f"'{w}'" for w in bad[:3]))
                serious = True
            if any(has_word(n, w) for w in OUTFIT_WORDS):
                h.append("name suggests more than one garment")
                serious = True
            if not c:
                serious = True  # not checked yet
            if c.get("error"):
                h.append("couldn't read the mesh: " + c["error"][:80])
                serious = True
            if c.get("compressed"):
                h.append("compressed mesh (Draco/meshopt): shape not checked")
                serious = True
            tris = c.get("tris")
            if tris is not None and not c.get("compressed"):
                if tris < 300:
                    h.append(f"very few triangles ({tris})")
                    serious = True
                if r["type"] in NEEDS_OPENINGS and c.get("big_openings", 0) == 0 and c.get("handles", 0) == 0:
                    h.append("closed shape, no openings (solid? no sleeve/neck/waist holes)")
                    serious = True
                if c.get("big_pieces", 0) >= 3:
                    h.append(f"{c['big_pieces']} big separate pieces (outfit/set, or a body inside?)")
                    serious = True
                size = c.get("size") or []
                if len(size) == 3 and max(size) > 0 and min(size) < 0.01 * max(size):
                    h.append("flat like a card or plane")
                    serious = True
            if c and c.get("textures", 1) == 0:
                h.append("no texture")
            if r.get("flag") or c.get("skins") or c.get("animations"):
                h.append("rigged/animated: may include a body")
                serious = True
            dups = [u for u in by_hash.get(c.get("sha1", ""), []) if u != uid]
            if not dups and tris and c.get("size"):
                key = (tris, tuple(round(x, 2) for x in sorted(c["size"])))
                dups = [u for u in by_shape.get(key, []) if u != uid]
            if dups:
                other = self.records.get(dups[0], {})
                h.append(f"duplicate of '{other.get('name', '?')}' ({other.get('file', '?')})")
                serious = True
            a = self.ai.get(uid)
            if a:
                if a["type"] != r["type"] and a["cloth"] >= 0.5 and a["type_p"] >= 0.6:
                    h.append(f"AI thinks this is {a['type']} ({pct(a['type_p'])}), not {r['type']}")
                    serious = True
                if a["body"] >= 0.35:
                    h.append(f"AI sees a body, mannequin or character ({pct(a['body'])}): maybe Edit")
                    serious = True
                if a["other"] >= 0.35:
                    h.append(f"AI thinks it may not be clothing (looks like {a['top_other']}, {pct(a['other'])})")
                    serious = True
            out[uid] = (h, serious)
        return out

    # ---- learning from your choices ----
    def features(self, uid: str, serious: bool) -> np.ndarray | None:
        a = self.ai.get(uid)
        if not a or "emb" not in a:
            return None
        if "_e" not in a:
            a["_e"] = np.frombuffer(base64.b64decode(a["emb"]), np.float16).astype(np.float32)
        c = self.checks.get(uid, {})
        extra = [a["cloth"], a["body"], a["other"], math.log1p(c.get("tris") or 0) / 10,
                 float((c.get("big_openings") or 0) > 0), float((c.get("handles") or 0) > 0),
                 math.log1p(c.get("pieces") or 0), float((c.get("textures") or 0) > 0), float(serious)]
        return np.concatenate([a["_e"], np.array(extra, np.float32)])

    def learn(self, hints: dict) -> None:
        feats = {}
        for uid in self.records:
            f = self.features(uid, hints.get(uid, ([], True))[1])
            if f is not None:
                feats[uid] = f
        labels = {}
        for uid, d in self.decisions.items():
            if uid in feats and not d.get("note", "").startswith("auto") and d.get("decision") in ("keep", "reject"):
                labels[uid] = 1 if d["decision"] == "keep" else 0
        nk = sum(labels.values())
        nr = len(labels) - nk
        if nk < 25 or nr < 25:
            self.keep_chance, self.learn_ok = {}, False
            self.learn_info = (f"Keep chance: needs at least 25 Keep and 25 Reject from you on models that have "
                               f"pictures (now {nk} and {nr}).")
            return
        # Squash the 768 AI numbers down to 64 so a few hundred choices are enough to learn from.
        uids = list(feats)
        x = np.stack([feats[u] for u in uids])
        emb_n = len(x[0]) - 9
        e = x[:, :emb_n] - x[:, :emb_n].mean(0)
        _, _, vt = np.linalg.svd(e, full_matrices=False)
        x = np.c_[e @ vt[:64].T, x[:, emb_n:]]
        chance, acc = learn_keep_chance({u: x[i] for i, u in enumerate(uids)}, labels)
        self.keep_chance = chance
        self.learn_ok = acc >= 0.75
        self.learn_info = (f"Keep chance learned from your {nk} Keep and {nr} Reject: right {round(acc * 100)}% "
                           f"of the time on models it didn't learn from"
                           + ("." if self.learn_ok else " (too low, so it only shows, it doesn't auto-sort yet)."))

    # ---- the three piles ----
    def auto_sort(self, opt: argparse.Namespace) -> str:
        with self.lock:
            hints = self.hints()
            self.learn(hints)
            # Exact copies: keep one per group (yours if you kept one), auto-reject the rest.
            groups: dict[str, list[str]] = {}
            for uid, c in self.checks.items():
                if uid in self.records and c.get("sha1") and self.path_of(self.records[uid]).is_file():
                    groups.setdefault(c["sha1"], []).append(uid)
            copy_of: dict[str, str] = {}
            for g in groups.values():
                if len(g) > 1:
                    g.sort(key=lambda u: (self.decisions.get(u, {}).get("decision") != "keep"
                                          or self.decisions.get(u, {}).get("note", "").startswith("auto"), u))
                    for u in g[1:]:
                        copy_of[u] = g[0]
            changed = 0
            for uid, r in self.records.items():
                d = self.decisions.get(uid)
                if d and not d.get("note", "").startswith("auto"):
                    continue  # you decided this one yourself: leave it
                c = self.checks.get(uid, {})
                a = self.ai.get(uid)
                kc = self.keep_chance.get(uid) if self.learn_ok else None
                h, serious = hints.get(uid, ([], True))
                name = r.get("name", "")
                decision, why = "", ""
                if "roblox" in name.lower():
                    decision, why = "reject", "auto: Roblox"
                elif LOW_POLY.search(name):
                    decision, why = "reject", "auto: low poly in the name"
                elif c.get("tris") is not None and not c.get("compressed") and c["tris"] < opt.min_tris:
                    decision, why = "reject", f"auto: only {c['tris']} triangles (under {opt.min_tris})"
                elif uid in copy_of:
                    other = self.records[copy_of[uid]].get("name", "?")
                    decision, why = "reject", f"auto: exact copy of '{other}'"
                elif a and a["other"] >= opt.not_clothing:
                    decision, why = "reject", f"auto: AI says not clothing (looks like {a['top_other']}, {pct(a['other'])})"
                elif a and a["cloth"] < 0.5 and any(x.startswith("name suggests not clothing") for x in h):
                    decision, why = "reject", f"auto: name and AI both say not clothing (clothing only {pct(a['cloth'])})"
                elif kc is not None and kc <= opt.sure_reject:
                    decision, why = "reject", f"auto: very like the ones you rejected (keep chance {pct(kc)})"
                elif (opt.auto_keep and a and not serious and a["type"] == r["type"] and a["body"] < 0.3
                      and a["cloth"] >= (0.85 if kc is not None else 0.92)
                      and (kc is None or kc >= opt.sure_keep)):
                    decision = "keep"
                    why = f"auto-keep: AI sure it's {r['type']} ({pct(a['cloth'])} clothing)"
                    if kc is not None:
                        why += f", keep chance {pct(kc)}"
                if decision:
                    if not d or d.get("note") != why or d.get("decision") != decision:
                        self.decisions[uid] = {"uid": uid, "decision": decision, "type": r["type"], "note": why,
                                               "name": name, "file": r["file"],
                                               "when": time.strftime("%Y-%m-%d %H:%M")}
                        changed += 1
                elif d:  # was auto-sorted before, but no longer sure
                    self.decisions.pop(uid)
                    changed += 1
            if changed:
                self._save_decisions()
            ak = sum(1 for d in self.decisions.values() if d.get("note", "").startswith("auto-keep"))
            ar = sum(1 for d in self.decisions.values()
                     if d.get("note", "").startswith("auto:") and d.get("decision") == "reject")
            todo = sum(1 for u in self.records if u not in self.decisions and self.path_of(self.records[u]).is_file())
            no_ai = sum(1 for u in self.records if u not in self.ai)
            self.sort_info = (f"Auto-kept {ak}, auto-rejected {ar}, needs you {todo}."
                              + (f" ({no_ai} models have no AI check yet.)" if no_ai else ""))
            self.sorts += 1
            return self.sort_info

    def items(self) -> list[dict]:
        hints = self.hints()
        out = []
        for uid, r in self.records.items():
            d = self.decisions.get(uid, {})
            c = self.checks.get(uid, {})
            a = self.ai.get(uid)
            t = self.thumb_path(uid)
            thumb = f"/thumbs/{t.name}?v={int(t.stat().st_mtime)}" if self.has_thumb(uid) else ""
            out.append({
                "uid": uid, "name": r.get("name", ""), "type": r["type"], "file": r["file"].replace("\\", "/"),
                "url": r.get("url", ""), "author": r.get("author", ""), "licence": r.get("licence", ""),
                "tris": c.get("tris"), "openings": c.get("big_openings"), "handles": c.get("handles"),
                "pieces": c.get("pieces"), "mb": round(r.get("bytes", 0) / 1e6, 1),
                "hints": hints.get(uid, ([], False))[0], "decision": d.get("decision", ""), "note": d.get("note", ""),
                "new_type": d.get("type", "") or r["type"],
                "exists": self.path_of(r).is_file(), "thumb": thumb,
                "ai": None if not a else {"cloth": a["cloth"], "body": a["body"], "other": a["other"],
                                          "type": a["type"], "type_p": a["type_p"], "top": a["top"]},
                "kc": self.keep_chance.get(uid),
            })
        out.sort(key=lambda x: (x["type"], x["name"].lower()))
        return out

    def status(self) -> dict:
        have = [u for u, r in self.records.items() if self.path_of(r).is_file()]
        w = self.ai_worker
        return {"models": len(have), "thumbs": sum(1 for u in have if self.has_thumb(u)),
                "thumb_failed": sum(1 for u in have if u in self.thumb_failed),
                "ai": sum(1 for u in have if u in self.ai), "ai_state": w.state if w else "off",
                "ai_queue": len(w.queue) if w else 0, "learn": self.learn_info, "sort": self.sort_info,
                "sorts": self.sorts}

    # ---- decisions ----
    def decide(self, uid: str, decision: str, new_type: str, note: str = "") -> None:
        if uid not in self.records or decision not in ("keep", "reject", "edit", ""):
            raise ValueError("bad decision")
        if new_type and new_type not in TYPES:
            raise ValueError("bad type")
        with self.lock:
            r = self.records[uid]
            if decision == "" and not new_type:
                self.decisions.pop(uid, None)
            else:
                self.decisions[uid] = {"uid": uid, "decision": decision, "type": new_type or r["type"],
                                       "note": note, "name": r.get("name", ""), "file": r["file"],
                                       "when": time.strftime("%Y-%m-%d %H:%M")}
            self._save_decisions()

    def _save_decisions(self) -> None:
        tmp = self.root / "review.csv.tmp"
        with open(tmp, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=["uid", "decision", "type", "note", "name", "file", "when"])
            w.writeheader()
            for row in self.decisions.values():
                w.writerow({k: row.get(k, "") for k in w.fieldnames})
        os.replace(tmp, self.root / "review.csv")

    # ---- apply: move files to match the decisions ----
    def apply(self) -> str:
        moved = 0
        missing = 0
        with self.lock:
            for uid, r in self.records.items():
                d = self.decisions.get(uid, {})
                typ = d.get("type") or r["type"]
                old = self.path_of(r)
                if not old.is_file():
                    missing += 1
                    continue
                flagged = bool(r.get("flag"))
                if d.get("decision") == "reject":
                    folder = Path("_rejected") / typ
                elif d.get("decision") == "edit":
                    folder = Path("_edit") / typ
                    r["edited"] = True  # CC BY: the credits must say it was changed
                elif flagged and d.get("decision") != "keep":
                    folder = Path("_check") / typ
                else:
                    folder = Path(typ)
                new_rel = folder / old.name
                review = {"reject": "rejected", "edit": "edit", "keep": "kept"}.get(d.get("decision", ""), "")
                if new_rel.as_posix() != r["file"].replace("\\", "/"):
                    (self.root / folder).mkdir(parents=True, exist_ok=True)
                    shutil.move(str(old), str(self.root / new_rel))
                    oj = old.with_suffix(".json")
                    if oj.is_file():
                        shutil.move(str(oj), str((self.root / new_rel).with_suffix(".json")))
                    moved += 1
                r["file"] = str(new_rel).replace("/", "\\")
                r["type"] = typ
                r["review"] = review
                j = (self.root / new_rel).with_suffix(".json")
                if j.is_file():
                    try:
                        meta = json.loads(j.read_text(encoding="utf-8"))
                        meta.update({"file": r["file"], "type": typ, "review": review,
                                     "edited": bool(r.get("edited"))})
                        j.write_text(json.dumps(meta, indent=2), encoding="utf-8")
                    except Exception:  # noqa: BLE001
                        pass
            # Rewrite done.jsonl (the collector reads it) in one go.
            tmp = self.state / "done.jsonl.tmp"
            with open(tmp, "w", encoding="utf-8") as f:
                for r in self.records.values():
                    f.write(json.dumps(r) + "\n")
            shutil.copy2(self.state / "done.jsonl", self.state / "done.jsonl.bak")
            os.replace(tmp, self.state / "done.jsonl")
            self.write_credits()
        rej = sum(1 for r in self.records.values() if r.get("review") == "rejected")
        ed = sum(1 for r in self.records.values() if r.get("review") == "edit")
        kept = len(self.records) - rej - ed
        msg = (f"Moved {moved} files. {kept} models in the dataset, {ed} waiting in _edit, "
               f"{rej} rejected (in _rejected).")
        if missing:
            msg += f" {missing} files were missing."
        return msg

    def write_credits(self) -> None:
        kept = sorted((r for r in self.records.values() if r.get("review") != "rejected"),
                      key=lambda r: (r["type"], r["name"].lower()))
        with open(self.root / "credits.csv", "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(["type", "file", "title", "author", "author_url", "licence", "licence_url", "model_url", "flag",
                        "changed"])
            for r in kept:
                w.writerow([r["type"], r["file"], r["name"], r["author"], r.get("author_url", ""), r["licence"],
                            r.get("licence_url", ""), r["url"], r.get("flag", ""),
                            "extra parts removed" if r.get("edited") else ""])
        lines = [
            "# Credits: ThreadSmith clothing training data",
            "",
            "These 3D models are used to train ThreadSmith's clothing LoRA. Each is under the licence shown.",
            "CC BY 4.0 models: https://creativecommons.org/licenses/by/4.0/ . Models marked \"changed\" had",
            "extra parts (such as a mannequin or character) removed; all others are unchanged. All were",
            "converted into training data. Sources: Sketchfab, via the Objaverse",
            "dataset (allenai/objaverse, ODC-By 1.0) or the Sketchfab API.",
            "",
        ]
        current = None
        for r in kept:
            if r["type"] != current:
                current = r["type"]
                lines += ["", f"## {current}", ""]
            author = f"[{r['author']}]({r['author_url']})" if r.get("author_url") else r.get("author", "unknown")
            changed = " (changed: extra parts removed)" if r.get("edited") else ""
            lines.append(f"- \"[{r['name']}]({r['url']})\" by {author}, {r['licence']}{changed}")
        (self.root / "CREDITS.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


# ---------------------------------------------------------------------------
# AI check (CLIP), running in the background while you review
# ---------------------------------------------------------------------------


def _feat(x):
    """transformers returns a tensor in v4 and an output object in some newer versions."""
    import torch
    if torch.is_tensor(x):
        return x
    for k in ("image_embeds", "text_embeds", "pooler_output"):
        v = getattr(x, k, None)
        if v is not None:
            return v
    return x[0]


class AIChecker:
    def __init__(self, ds: Dataset, opt: argparse.Namespace):
        self.ds, self.opt = ds, opt
        self.queue: list[str] = []
        self.cv = threading.Condition()
        self.enabled = not opt.no_ai
        self.state = "off (--no-ai)" if not self.enabled else "waiting for pictures"
        self.model = None
        if self.enabled:
            threading.Thread(target=self._run, daemon=True).start()

    def add(self, uid: str) -> None:
        if not self.enabled:
            return
        with self.cv:
            if uid not in self.queue:
                self.queue.append(uid)
            self.cv.notify()

    def _load(self) -> None:
        self.state = "loading the AI model (the first time it downloads about 1.7 GB)"
        import torch
        from transformers import AutoTokenizer, CLIPImageProcessor, CLIPModel
        self.torch = torch
        self.dev = "cuda" if torch.cuda.is_available() else "cpu"
        self.dtype = torch.float16 if self.dev == "cuda" else torch.float32
        print(f"AI check: loading {self.opt.ai_model} on {self.dev.upper()}...")
        self.model = CLIPModel.from_pretrained(self.opt.ai_model, torch_dtype=self.dtype).to(self.dev).eval()
        self.proc = CLIPImageProcessor.from_pretrained(self.opt.ai_model)
        tok = AutoTokenizer.from_pretrained(self.opt.ai_model)
        labels, kinds, types = all_labels()
        with torch.no_grad():
            embs = []
            for tpl in PROMPT_TEMPLATES:
                t = tok([tpl.format(lab) for lab in labels], padding=True, return_tensors="pt")
                e = _feat(self.model.get_text_features(input_ids=t["input_ids"].to(self.dev),
                                                       attention_mask=t["attention_mask"].to(self.dev))).float()
                embs.append(e / e.norm(dim=-1, keepdim=True))
            tm = torch.stack(embs).mean(0)
            tm = tm / tm.norm(dim=-1, keepdim=True)
        self.text = tm.cpu().numpy()
        self.scale = float(self.model.logit_scale.exp().float().item())
        self.labels, self.kinds, self.types = labels, np.array(kinds), np.array(types)
        print("AI check: ready.")

    def _summary(self, e: np.ndarray) -> dict:
        z = self.scale * (self.text @ e)
        p = np.exp(z - z.max())
        p /= p.sum()
        cloth = float(p[self.kinds == "cloth"].sum())
        tp = {t: float(p[self.types == t].sum()) for t in TYPES}
        best_t = max(tp, key=tp.get)
        oth = np.where(self.kinds == "other")[0]
        return {"cloth": round(cloth, 4), "body": round(float(p[self.kinds == "body"].sum()), 4),
                "other": round(float(p[oth].sum()), 4), "type": best_t,
                "type_p": round(tp[best_t] / max(cloth, 1e-9), 4), "top": self.labels[int(p.argmax())],
                "top_other": self.labels[int(oth[p[oth].argmax()])]}

    def _check(self, uids: list[str]) -> None:
        from PIL import Image
        torch = self.torch
        imgs, ok = [], []
        for uid in uids:
            t = self.ds.thumb_path(uid)
            try:
                im = Image.open(t).convert("RGB")
                mtime = t.stat().st_mtime
            except Exception:  # noqa: BLE001
                continue
            w, h = im.size
            tw, th = w // 2, h // 2
            imgs += [im.crop((x * tw, y * th, (x + 1) * tw, (y + 1) * th)) for y in range(2) for x in range(2)]
            ok.append((uid, mtime))
        if not ok:
            return
        with torch.no_grad():
            px = self.proc(images=imgs, return_tensors="pt")["pixel_values"].to(self.dev, self.dtype)
            e = _feat(self.model.get_image_features(pixel_values=px)).float()
            e = e / e.norm(dim=-1, keepdim=True)
            e = e.view(len(ok), 4, -1).mean(1)
            e = (e / e.norm(dim=-1, keepdim=True)).cpu().numpy()
        key = labels_key()
        for (uid, mtime), emb in zip(ok, e):
            rec = {"uid": uid, "v": AI_VERSION, "model": self.opt.ai_model, "labels": key, "thumb_mtime": mtime}
            rec.update(self._summary(emb))
            rec["emb"] = base64.b64encode(emb.astype(np.float16).tobytes()).decode()
            self.ds.append_jsonl("ai.jsonl", rec)
            self.ds.ai[uid] = rec

    def _run(self) -> None:
        since_sort, last_sort = 0, time.time()
        while True:
            with self.cv:
                while not self.queue:
                    self.state = "ready" if self.model is not None else self.state
                    # Sort with the new results once things go quiet (not after every single picture).
                    if since_sort and time.time() - last_sort > 45:
                        break
                    self.cv.wait(timeout=5)
                batch = self.queue[:8]
                del self.queue[:8]
            if not batch:
                print("AI check: " + self.ds.auto_sort(self.opt))
                since_sort, last_sort = 0, time.time()
                continue
            try:
                if self.model is None:
                    self._load()
                self.state = "checking"
                self._check(batch)
                since_sort += len(batch)
                if since_sort >= 400:
                    print("AI check: " + self.ds.auto_sort(self.opt))
                    since_sort, last_sort = 0, time.time()
            except ImportError as exc:
                self.state = f"off: {exc.name or exc} isn't installed in this Python (run it with ComfyUI's Python)"
                print("AI check is " + self.state)
                self.enabled = False
                with self.cv:
                    self.queue.clear()
                return
            except Exception as exc:  # noqa: BLE001
                self.state = f"error: {type(exc).__name__}: {exc}"[:300]
                print("AI check stopped: " + self.state)
                if self.model is None:
                    self.enabled = False
                    with self.cv:
                        self.queue.clear()
                    return
                time.sleep(2)


# ---------------------------------------------------------------------------
# Review page (local only: 127.0.0.1)
# ---------------------------------------------------------------------------

PAGE = r"""<!doctype html>
<html><head><meta charset="utf-8"><title>ThreadSmith - Review clothing</title>
<script type="module" src="https://ajax.googleapis.com/ajax/libs/model-viewer/3.5.0/model-viewer.min.js"
  onerror="var s=document.createElement('script');s.type='module';s.src='https://unpkg.com/@google/model-viewer/dist/model-viewer.min.js';document.head.appendChild(s)"></script>
<style>
 :root{--bg:#16181c;--card:#22252b;--line:#33373f;--text:#e6e6e6;--dim:#9aa0a8;--keep:#3fa66b;--rej:#d0504a;--warn:#e0a040;--edit:#a070e0;--sel:#4d8df0;--ai:#5fb3d9}
 *{box-sizing:border-box} body{margin:0;background:var(--bg);color:var(--text);font:14px system-ui,Segoe UI,sans-serif}
 header{position:sticky;top:0;z-index:5;background:#1c1f24;border-bottom:1px solid var(--line);padding:8px 12px;display:flex;flex-wrap:wrap;gap:10px;align-items:center}
 header b{font-size:15px} select,button,input{background:#2b2f36;color:var(--text);border:1px solid var(--line);border-radius:6px;padding:5px 8px;font:inherit}
 button{cursor:pointer} button:hover{border-color:var(--sel)} .stats{color:var(--dim)} .grow{flex:1}
 .help{color:var(--dim);font-size:12px;padding:6px 12px;border-bottom:1px solid var(--line)}
 #work{font-size:12px;padding:6px 12px;border-bottom:1px solid var(--line);display:flex;gap:12px;align-items:center;flex-wrap:wrap;color:var(--dim)}
 #work .bar{width:180px;height:6px;background:#2b2f36;border-radius:3px;overflow:hidden} #work .bar i{display:block;height:100%;background:var(--ai);width:0}
 #refresh{display:none;background:var(--ai);color:#111;border-color:var(--ai)}
 kbd{background:#2b2f36;border:1px solid var(--line);border-radius:4px;padding:0 4px;font-size:11px}
 #grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(250px,1fr));gap:10px;padding:12px 12px 320px}
 .card{background:var(--card);border:2px solid var(--line);border-radius:10px;overflow:hidden;display:flex;flex-direction:column}
 .card.sel{border-color:var(--sel);box-shadow:0 0 0 2px var(--sel)} .card.keep{border-color:var(--keep)} .card.reject{border-color:var(--rej);opacity:.5}
 .card.edit{border-color:var(--edit)}
 .card.keep.sel,.card.reject.sel,.card.edit.sel{box-shadow:0 0 0 3px var(--sel)}
 .pic{position:relative;width:100%;aspect-ratio:1;background:#e9e9e9;cursor:pointer}
 .pic img{width:100%;height:100%;display:block} .pic model-viewer{width:100%;height:100%;background:#e9e9e9}
 .pic .b3d{position:absolute;right:6px;bottom:6px;padding:2px 7px;font-size:12px;opacity:.85}
 .card.reject .pic::after{content:"\2715";position:absolute;inset:0;display:flex;align-items:center;justify-content:center;font-size:90px;color:var(--rej);pointer-events:none}
 .info{padding:7px 9px;display:flex;flex-direction:column;gap:3px}
 .name{font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
 .meta{color:var(--dim);font-size:12px} .meta a{color:var(--dim)}
 .hint{color:var(--warn);font-size:12px} .ok{color:var(--keep);font-size:12px} .ai{color:var(--ai);font-size:12px}
 .ai.bad{color:var(--rej)}
 .row{display:flex;gap:5px;margin-top:4px} .row button{flex:1;padding:4px}
 .bk.on{background:var(--keep);border-color:var(--keep)} .br.on{background:var(--rej);border-color:var(--rej)}
 .be.on{background:var(--edit);border-color:var(--edit)} .note{color:var(--dim);font-size:12px;font-style:italic}
 .bai{margin-top:3px;font-size:12px;padding:3px;color:var(--ai)}
 .badge{font-size:11px;padding:1px 6px;border-radius:9px;background:#2b2f36;border:1px solid var(--line)}
 #msg{position:fixed;bottom:14px;left:50%;transform:translateX(-50%);background:#2b2f36;border:1px solid var(--line);padding:8px 14px;border-radius:8px;display:none;z-index:9}
 .pager{display:flex;gap:6px;align-items:center}
 #shots{position:fixed;right:10px;bottom:10px;z-index:8;background:#1c1f24;border:1px solid var(--line);border-radius:10px;padding:8px;display:none}
 #shots .t{font-size:12px;color:var(--dim);margin-bottom:6px;max-width:500px}
 #shots iframe{width:496px;height:244px;border:0;display:block;background:#1c1f24}
</style></head><body>
<header>
 <b>Review clothing</b>
 <label>Type <select id="ftype"></select></label>
 <label>Show <select id="fshow">
   <option value="todo">Needs you (not sure)</option><option value="autoreject">Auto-rejected</option>
   <option value="autokeep">Auto-kept</option><option value="all">All</option>
   <option value="keep">Kept (all)</option><option value="edit">To edit</option><option value="reject">Rejected (all)</option></select></label>
 <label>Sort <select id="fsort"><option value="ai">AI: least clothing-like first</option><option value="kc">Keep chance (low first)</option>
   <option value="hints">Most problems first</option><option value="name">Name</option><option value="tris">Triangles</option></select></label>
 <label>Per page <select id="fsize"><option>24</option><option selected>48</option><option>96</option></select></label>
 <input id="fsearch" placeholder="Search name" size="14">
 <div class="pager"><button id="prev">&#9664;</button><span id="pinfo"></span><button id="next">&#9654;</button></div>
 <span class="grow"></span>
 <span class="stats" id="stats"></span>
 <button id="resort" title="Learn from your latest choices and sort the not-reviewed models again">Re-sort with AI</button>
 <button id="keepRest" title="Keep every model on this page that isn't marked yet (Shift+K)">Keep rest of page</button>
 <button id="apply" title="Move files to match the decisions and rewrite the credits">Apply decisions</button>
</header>
<div id="work"><span id="wpics"></span><span class="bar"><i id="wbar"></i></span><span id="wai"></span>
 <button id="pause">Pause pictures</button><button id="refresh">AI sorted more models: refresh list</button>
 <span id="wlearn"></span></div>
<div class="help">Fast way: look over the page, <b>click the picture</b> of each bad one (marks Reject), then <kbd>Shift</kbd>+<kbd>K</kbd> keeps the rest and brings in more.
 Keys for the selected model: <kbd>K</kbd> keep &nbsp;<kbd>X</kbd> reject &nbsp;<kbd>E</kbd> edit (good garment + extra stuff) &nbsp;<kbd>A</kbd> keep as the AI's type &nbsp;<kbd>U</kbd> undo mark &nbsp;<kbd>T</kbd> change type &nbsp;<kbd>V</kbd> 3D view &nbsp;
 <kbd>&larr;</kbd><kbd>&rarr;</kbd> move &nbsp;<kbd>N</kbd>/<kbd>P</kbd> next/previous page. Orange lines and blue AI lines are guesses: you decide.</div>
<div id="grid"></div><div id="msg"></div>
<div id="shots"><div class="t" id="shotsText">Making pictures. Keep this tab open in front (a hidden tab pauses it). You can review while it works.</div><iframe id="shotsFrame" title="picture maker"></iframe></div>
<script>
const TYPES = __TYPES__;
let items = [], view = [], page = 0, sel = 0, lastSorts = null;
const $ = id => document.getElementById(id);
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const pc = x => x == null ? '?' : Math.round(x*100) + '%';
const sleep = ms => new Promise(r => setTimeout(r, ms));
function toast(t){ const m=$('msg'); m.textContent=t; m.style.display='block'; clearTimeout(m._t); m._t=setTimeout(()=>m.style.display='none',4000); }
const isAuto = i => (i.note||'').startsWith('auto');
async function load(){
  items = await (await fetch('/api/items')).json();
  const ft = $('ftype'); let cur = ft.value; if (cur === '' && !ft.options.length) { try { cur = localStorage.tsType || ''; } catch(e) {} }
  ft.innerHTML = '<option value="">All types</option>' + TYPES.map(t => {
    const n = items.filter(i => i.type === t && i.exists).length; return `<option value="${t}">${t} (${n})</option>`; }).join('');
  ft.value = cur; filter(true);
  $('refresh').style.display = 'none';
  try { lastSorts = (await (await fetch('/api/status')).json()).sorts; } catch(e) {}
}
function filter(keepPage){
  const t = $('ftype').value, s = $('fshow').value, q = $('fsearch').value.toLowerCase(), so = $('fsort').value;
  try { localStorage.tsType = t; } catch(e) {}
  view = items.filter(i => i.exists && (!t || i.type === t) && (!q || i.name.toLowerCase().includes(q)) && (
    s === 'all' || (s === 'todo' && !i.decision) || (s === 'autoreject' && isAuto(i) && i.decision === 'reject') ||
    (s === 'autokeep' && isAuto(i) && i.decision === 'keep') || i.decision === s));
  const cl = i => i.ai ? i.ai.cloth : 2;
  if (so === 'ai') view.sort((a,b) => cl(a) - cl(b) || a.name.localeCompare(b.name));
  else if (so === 'kc') view.sort((a,b) => (a.kc ?? 2) - (b.kc ?? 2));
  else if (so === 'hints') view.sort((a,b) => b.hints.length - a.hints.length || a.name.localeCompare(b.name));
  else if (so === 'tris') view.sort((a,b) => (a.tris||0) - (b.tris||0));
  else view.sort((a,b) => a.name.localeCompare(b.name));
  if (!keepPage) { page = 0; sel = 0; }
  draw();
}
function per(){ return +$('fsize').value; }
function pageItems(){ return view.slice(page*per(), page*per()+per()); }
function viewer(i){ return `<model-viewer src="/files/${encodeURI(i.file)}" loading="lazy" camera-controls auto-rotate
  rotation-per-second="40deg" shadow-intensity="0.6" exposure="1.1" interaction-prompt="none"></model-viewer>`; }
function aiLine(i){
  if (!i.ai) return i.thumb ? '<div class="ai">AI: waiting to check</div>' : '<div class="ai">AI: needs a picture first</div>';
  const a = i.ai, bad = a.cloth < 0.5;
  let s = `AI: ${pc(a.cloth)} clothing · looks like ${esc(a.top)}`;
  if (a.cloth >= 0.5) s += ` · ${esc(a.type)} ${pc(a.type_p)}`;
  if (i.kc != null) s += ` · keep chance ${pc(i.kc)}`;
  return `<div class="ai ${bad?'bad':''}">${s}</div>`;
}
function draw(){
  const pages = Math.max(1, Math.ceil(view.length / per())); page = Math.min(page, pages-1);
  $('pinfo').textContent = `page ${page+1} / ${pages} (${view.length})`;
  stats();
  const g = $('grid'); g.innerHTML = '';
  pageItems().forEach((i, n) => {
    const c = document.createElement('div');
    c.className = 'card' + (i.decision ? ' ' + i.decision : '') + (n === sel ? ' sel' : '');
    const typeSel = TYPES.map(t => `<option ${t===i.new_type?'selected':''}>${t}</option>`).join('');
    const nums = i.tris == null ? 'not checked' : `${(i.tris/1000).toFixed(1)}k tris · ${i.openings ?? '?'} openings` +
      (i.handles ? ` · ${i.handles} tunnels` : '') + ` · ${i.pieces ?? '?'} pieces · ${i.mb} MB`;
    const useAi = i.ai && i.ai.cloth >= 0.5 && i.ai.type !== i.new_type;
    c.innerHTML = `<div class="pic">${i.thumb ? `<img src="${i.thumb}" loading="lazy" alt="">` : viewer(i)}
        ${i.thumb ? '<button class="b3d" title="Spin it in 3D (V)">3D</button>' : ''}</div>
      <div class="info"><div class="name" title="${esc(i.name)}">${esc(i.name)}</div>
      <div class="meta"><span class="badge">${esc(i.type)}</span> ${nums}</div>
      <div class="meta">${esc(i.licence)} · ${esc(i.author)} · <a href="${esc(i.url)}" target="_blank">Sketchfab page</a></div>
      ${aiLine(i)}
      ${i.note ? `<div class="note">${esc(i.note)}</div>` : ''}
      ${i.hints.length ? i.hints.map(h => `<div class="hint">&#9888; ${esc(h)}</div>`).join('') : '<div class="ok">no problems found</div>'}
      <div class="row"><button class="bk ${i.decision==='keep'?'on':''}">Keep</button>
        <button class="br ${i.decision==='reject'?'on':''}">Reject</button>
        <button class="be ${i.decision==='edit'?'on':''}" title="Good garment, but has a mannequin/character/extra parts">Edit</button>
        <select title="Garment type">${typeSel}</select></div>
      ${useAi ? `<button class="bai" title="Keep it, with the type the AI thinks (A)">Keep as ${esc(i.ai.type)} (AI's guess)</button>` : ''}</div>`;
    c.addEventListener('click', e => { if (e.target.closest('button,select,a,model-viewer')) return;
      sel = n; mark();
      if (e.target.closest('.pic')) decide(i, i.decision === 'reject' && !isAuto(i) ? '' : 'reject'); });
    c.querySelector('.bk').onclick = () => { sel = n; decide(i, i.decision==='keep' && !isAuto(i) ? '' : 'keep'); };
    c.querySelector('.br').onclick = () => { sel = n; decide(i, i.decision==='reject' && !isAuto(i) ? '' : 'reject'); };
    c.querySelector('.be').onclick = () => { sel = n; decide(i, i.decision==='edit' ? '' : 'edit'); };
    c.querySelector('select').onchange = e => { sel = n; decide(i, i.decision || 'keep', e.target.value); };
    const b3 = c.querySelector('.b3d'); if (b3) b3.onclick = () => { sel = n; show3d(c, i); };
    const ba = c.querySelector('.bai'); if (ba) ba.onclick = () => { sel = n; decide(i, 'keep', i.ai.type); };
    g.appendChild(c);
  });
}
function show3d(c, i){ const p = c.querySelector('.pic'); if (p.querySelector('model-viewer')) return; p.innerHTML = viewer(i); }
function mark(){ [...$('grid').children].forEach((c, n) => c.classList.toggle('sel', n === sel)); $('grid').children[sel]?.scrollIntoView({block:'nearest'}); }
async function decide(i, d, t){
  t = t || i.new_type;
  const r = await fetch('/api/decide', {method:'POST', body: JSON.stringify({uid:i.uid, decision:d, type: t===i.type && !d ? '' : t})});
  if (!r.ok) { toast('Could not save: ' + await r.text()); return; }
  i.decision = d; i.new_type = t; i.note = '';
  const c = $('grid').children[pageItems().indexOf(i)];
  if (c) { c.className = 'card' + (d ? ' ' + d : '') + (pageItems().indexOf(i) === sel ? ' sel' : '');
    c.querySelector('.bk').classList.toggle('on', d==='keep'); c.querySelector('.br').classList.toggle('on', d==='reject');
    c.querySelector('.be').classList.toggle('on', d==='edit'); c.querySelector('select').value = t;
    const nt = c.querySelector('.note'); if (nt) nt.remove(); }
  stats();
}
function stats(){
  const ex = items.filter(x => x.exists), n = f => ex.filter(f).length;
  $('stats').textContent = `Needs you ${n(x => !x.decision)} · auto-kept ${n(x => isAuto(x) && x.decision==='keep')} · auto-rejected ${n(x => isAuto(x) && x.decision==='reject')} · ` +
    `you: kept ${n(x => !isAuto(x) && x.decision==='keep')}, edit ${n(x => x.decision==='edit')}, rejected ${n(x => !isAuto(x) && x.decision==='reject')}`;
}
async function keepRest(){
  for (const i of pageItems()) if (!i.decision) await decide(i, 'keep');
  toast('Kept the rest of this page'); filter(true); window.scrollTo(0,0);
}
function go(d){ const pages = Math.ceil(view.length/per()); page = Math.max(0, Math.min(pages-1, page+d)); sel = 0; draw(); window.scrollTo(0,0); }
$('prev').onclick = () => go(-1); $('next').onclick = () => go(1);
['ftype','fshow','fsort','fsize'].forEach(id => $(id).onchange = () => filter());
$('fsearch').oninput = () => filter();
$('keepRest').onclick = keepRest;
$('refresh').onclick = load;
$('resort').onclick = async () => { $('resort').disabled = true; toast('Sorting...');
  const r = await fetch('/api/autosort', {method:'POST'}); toast(await r.text()); $('resort').disabled = false; load(); };
$('apply').onclick = async () => {
  if (!confirm('Move the files to match your decisions (auto ones too)?\nRejected models go to _rejected, Edit models go to _edit (nothing is deleted), and the credits are rewritten.')) return;
  const r = await fetch('/api/apply', {method:'POST'}); toast(await r.text()); load();
};
document.addEventListener('keydown', e => {
  if (e.target.matches('input,select') || e.ctrlKey || e.altKey || e.metaKey) return;
  const its = pageItems(), i = its[sel], k = e.key.toLowerCase();
  const next = () => { sel = Math.min(its.length-1, sel+1); mark(); };
  if (e.key === 'ArrowRight') { sel = Math.min(its.length-1, sel+1); mark(); e.preventDefault(); }
  else if (e.key === 'ArrowLeft') { sel = Math.max(0, sel-1); mark(); e.preventDefault(); }
  else if (k === 'k' && e.shiftKey) keepRest();
  else if (k === 'k' && i) { decide(i, 'keep'); next(); }
  else if ((k === 'x' || k === 'r') && i) { decide(i, 'reject'); next(); }
  else if (k === 'e' && i) { decide(i, 'edit'); next(); }
  else if (k === 'a' && i && i.ai) { decide(i, 'keep', i.ai.cloth >= 0.5 ? i.ai.type : i.new_type); next(); }
  else if (k === 'u' && i) decide(i, '');
  else if (k === 't' && i) $('grid').children[sel]?.querySelector('select').focus();
  else if (k === 'v' && i) show3d($('grid').children[sel], i);
  else if (k === 'n') go(1); else if (k === 'p') go(-1);
});

// ---------- pictures: made in their own little frame (/shots), so the grid's 3D views can't disturb them ----------
let shotDone = 0, shotTotal = 0, paused = false, t0 = 0;
async function startShots(){
  const todo = await (await fetch('/api/thumbs_todo')).json();
  if (!todo.length) return;
  $('shots').style.display = 'block'; $('shotsFrame').src = '/shots';
}
window.addEventListener('message', e => {
  if (e.origin !== location.origin) return;
  const m = e.data || {};
  if (m.kind === 'progress') { shotDone = m.done; shotTotal = m.total; t0 = m.t0; work(); }
  if (m.kind === 'finished') { $('shots').style.display = 'none'; $('shotsFrame').src = 'about:blank'; shotTotal = 0; work(); load(); }
});
$('pause').onclick = () => { paused = !paused; $('pause').textContent = paused ? 'Resume pictures' : 'Pause pictures';
  $('shotsFrame').contentWindow?.postMessage({kind: 'pause', paused}, location.origin); };
function work(){
  if (shotTotal) {
    const el = (Date.now() - t0) / 1000, left = shotDone ? el / shotDone * (shotTotal - shotDone) : 0;
    $('wpics').textContent = `Pictures: ${shotDone} / ${shotTotal} made this time` + (shotDone > 3 ? ` (about ${Math.ceil(left/60)} min left)` : '');
    $('wbar').style.width = (100 * shotDone / shotTotal) + '%';
    $('shotsText').textContent = `Making pictures ${shotDone} / ${shotTotal}. Keep this tab open in front (a hidden tab pauses it). You can review while it works.`;
  }
}
async function poll(){
  try {
    const s = await (await fetch('/api/status')).json();
    if (!shotTotal) { $('wpics').textContent = `Pictures: ${s.thumbs} / ${s.models}` + (s.thumb_failed ? ` (${s.thumb_failed} couldn't be made: they get the 3D view)` : '');
      $('wbar').style.width = (100 * s.thumbs / Math.max(1, s.models)) + '%'; }
    $('wai').textContent = `AI checked ${s.ai} / ${s.models} · AI: ${s.ai_state}` + (s.ai_queue ? ` (${s.ai_queue} waiting)` : '');
    $('wlearn').textContent = s.learn + (s.sort ? '  ' + s.sort : '');
    if (lastSorts !== null && s.sorts !== lastSorts) $('refresh').style.display = 'inline-block';
    lastSorts = s.sorts;
  } catch (e) {}
  setTimeout(poll, 3000);
}
load().then(() => { poll(); startShots(); });
</script></body></html>
"""


SHOTS_PAGE = r"""<!doctype html>
<html><head><meta charset="utf-8"><title>pictures</title>
<script type="module" src="https://ajax.googleapis.com/ajax/libs/model-viewer/3.5.0/model-viewer.min.js"
  onerror="var s=document.createElement('script');s.type='module';s.src='https://unpkg.com/@google/model-viewer/dist/model-viewer.min.js';document.head.appendChild(s)"></script>
<style>body{margin:0;background:#1c1f24;display:flex;gap:6px;overflow:hidden} model-viewer{width:240px;height:240px;background:#e9e9e9}</style>
</head><body>
<script>
const VIEWS = [[0, 78], [40, 68], [90, 80], [180, 78]];   // front, 3/4, side, back (degrees around, degrees down from the top)
const TILE = 320;
let shotTodo = [], shotDone = 0, shotTotal = 0, paused = false, t0 = 0, shotLock = Promise.resolve();
const sleep = ms => new Promise(r => setTimeout(r, ms));
const tell = m => parent.postMessage(m, location.origin);
function frames(n){ return new Promise(res => { let k = 0; const to = setTimeout(res, 120*n);
  const f = () => { if (++k >= n) { clearTimeout(to); res(); } else requestAnimationFrame(f); }; requestAnimationFrame(f); }); }
function loadInto(mv, src, ms){
  return new Promise((res, rej) => {
    const done = (ok, why) => { clearTimeout(t); mv.removeEventListener('load', onLoad); mv.removeEventListener('error', onErr); ok ? res() : rej(new Error(why)); };
    const onLoad = () => done(true), onErr = () => done(false, 'could not load the model');
    const t = setTimeout(() => done(false, 'timed out'), ms);
    mv.addEventListener('load', onLoad); mv.addEventListener('error', onErr);
    mv.src = src;
  });
}
async function shoot(mv, it){
  await loadInto(mv, '/files/' + encodeURI(it.file), 30000 + it.mb * 2000);
  // Viewers load models side by side, but take their shots one at a time
  // (they share one renderer, and shooting at the same time gives cropped pictures).
  const prev = shotLock; let release; shotLock = new Promise(r => release = r);
  await prev;
  try {
  await frames(2);
  if (mv.updateFraming) await mv.updateFraming();   // fit the camera to this model, not the last one
  await frames(2);
  const sheet = document.createElement('canvas'); sheet.width = sheet.height = TILE * 2;
  const g = sheet.getContext('2d'); g.fillStyle = '#e9e9e9'; g.fillRect(0, 0, TILE*2, TILE*2);
  for (let v = 0; v < 4; v++) {
    mv.cameraTarget = 'auto'; mv.fieldOfView = 'auto';
    mv.cameraOrbit = `${VIEWS[v][0]}deg ${VIEWS[v][1]}deg auto`;
    mv.jumpCameraToGoal();
    await frames(3);
    const b = await mv.toBlob({mimeType: 'image/png', idealAspect: false});
    const bmp = await createImageBitmap(b);
    const s = Math.min(TILE / bmp.width, TILE / bmp.height), w = bmp.width * s, h = bmp.height * s;
    g.drawImage(bmp, (v % 2) * TILE + (TILE - w) / 2, (v >> 1) * TILE + (TILE - h) / 2, w, h); bmp.close();
  }
  return await new Promise(r => sheet.toBlob(r, 'image/jpeg', 0.88));
  } finally { release(); }
}
async function shotWorker(mv){
  while (shotTodo.length) {
    while (paused) await sleep(400);
    const it = shotTodo.shift();
    try {
      const jpg = await shoot(mv, it);
      const r = await fetch('/api/thumb?uid=' + encodeURIComponent(it.uid), {method: 'POST', body: jpg});
      if (!r.ok) throw new Error(await r.text());
    } catch (err) {
      await fetch('/api/thumb_fail?uid=' + encodeURIComponent(it.uid), {method: 'POST', body: String(err && err.message || err)});
    }
    shotDone++; tell({kind: 'progress', done: shotDone, total: shotTotal, t0});
  }
}
async function startShots(){
  shotTodo = await (await fetch('/api/thumbs_todo')).json();
  shotTotal = shotTodo.length; shotDone = 0; t0 = Date.now();
  if (!shotTotal) return;
  tell({kind: 'progress', done: 0, total: shotTotal, t0});
  await customElements.whenDefined('model-viewer');
  // Never drop to a lower resolution when the computer is busy (that crops the pictures).
  const MV = customElements.get('model-viewer'); if (MV) MV.minimumRenderScale = 1;
  const vs = [0, 1].map(() => { const mv = document.createElement('model-viewer');
    for (const [k, v] of [['loading','eager'],['reveal','auto'],['shadow-intensity','0'],['exposure','1.1'],['interaction-prompt','none'],['environment-image','neutral']]) mv.setAttribute(k, v);
    document.body.appendChild(mv); return mv; });
  await Promise.all(vs.map(shotWorker));
  tell({kind: 'finished'});
}
window.addEventListener('message', e => { if (e.origin === location.origin && e.data && e.data.kind === 'pause') paused = e.data.paused; });
startShots();
</script></body></html>
"""


def make_handler(ds: Dataset, opt: argparse.Namespace):
    page = PAGE.replace("__TYPES__", json.dumps(TYPES)).encode("utf-8")
    shots_page = SHOTS_PAGE.encode("utf-8")
    root = ds.root.resolve()
    thumbs = ds.thumbs.resolve()

    class H(BaseHTTPRequestHandler):
        def log_message(self, *a):  # keep the window quiet
            pass

        def send(self, code, body: bytes, ctype="text/plain; charset=utf-8"):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store" if ctype.startswith(("application/json", "text")) else "max-age=3600")
            self.end_headers()
            self.wfile.write(body)

        def send_file(self, f: Path, ctype: str):
            self.send_response(200)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(f.stat().st_size))
            self.send_header("Cache-Control", "max-age=3600")
            self.end_headers()
            try:
                with open(f, "rb") as fh:
                    shutil.copyfileobj(fh, self.wfile, 1 << 20)
            except (ConnectionError, BrokenPipeError):
                pass

        def do_GET(self):
            path = urlparse(self.path).path
            if path == "/":
                return self.send(200, page, "text/html; charset=utf-8")
            if path == "/shots":
                return self.send(200, shots_page, "text/html; charset=utf-8")
            if path == "/api/items":
                return self.send(200, json.dumps(ds.items()).encode(), "application/json")
            if path == "/api/status":
                return self.send(200, json.dumps(ds.status()).encode(), "application/json")
            if path == "/api/thumbs_todo":
                return self.send(200, json.dumps(ds.thumbs_todo()).encode(), "application/json")
            if path.startswith("/thumbs/"):
                f = (thumbs / unquote(path[len("/thumbs/"):])).resolve()
                if f.parent != thumbs or f.suffix.lower() != ".jpg" or not f.is_file():
                    return self.send(404, b"not found")
                return self.send_file(f, "image/jpeg")
            if path.startswith("/files/"):
                rel = unquote(path[len("/files/"):])
                f = (root / rel).resolve()
                if root not in f.parents or f.suffix.lower() != ".glb" or not f.is_file():
                    return self.send(404, b"not found")
                return self.send_file(f, "model/gltf-binary")
            self.send(404, b"not found")

        def do_POST(self):
            u = urlparse(self.path)
            n = int(self.headers.get("Content-Length") or 0)
            if n > 8_000_000:
                return self.send(413, b"too big")
            body = self.rfile.read(n)
            uid = parse_qs(u.query).get("uid", [""])[0]
            try:
                if u.path == "/api/decide":
                    d = json.loads(body or b"{}")
                    ds.decide(d.get("uid", ""), d.get("decision", ""), d.get("type", ""))
                    return self.send(200, b"ok")
                if u.path == "/api/thumb":
                    ds.save_thumb(uid, body)
                    return self.send(200, b"ok")
                if u.path == "/api/thumb_fail":
                    ds.thumb_fail(uid, body.decode("utf-8", "replace"))
                    return self.send(200, b"ok")
                if u.path == "/api/autosort":
                    return self.send(200, ds.auto_sort(opt).encode())
                if u.path == "/api/apply":
                    return self.send(200, ds.apply().encode())
            except Exception as exc:  # noqa: BLE001
                return self.send(400, str(exc).encode())
            self.send(404, b"not found")

    return H


def main() -> int:
    p = argparse.ArgumentParser(description="Check and review collected clothing models")
    p.add_argument("root", help="The Training data folder")
    p.add_argument("--workers", type=int, default=max(1, min(4, (os.cpu_count() or 4) - 2)),
                   help="Models shape-checked at once")
    p.add_argument("--port", type=int, default=8765)
    p.add_argument("--min-tris", type=int, default=1000,
                   help="Auto-reject models with fewer triangles than this (default 1000)")
    p.add_argument("--skip-checks", action="store_true", help="Open the review page without checking new models")
    p.add_argument("--apply", action="store_true", help="Only move files to match review.csv, then stop")
    p.add_argument("--no-ai", action="store_true", help="Don't run the AI (CLIP) check")
    p.add_argument("--ai-model", default="openai/clip-vit-large-patch14",
                   help="Hugging Face CLIP model (or a local folder) for the AI check")
    p.add_argument("--no-auto-keep", dest="auto_keep", action="store_false",
                   help="Never auto-keep: only auto-reject, you look at everything else")
    p.add_argument("--not-clothing", type=float, default=0.80,
                   help="Auto-reject when the AI is at least this sure it's not clothing (default 0.80)")
    p.add_argument("--sure-keep", type=float, default=0.80,
                   help="Keep chance needed for an auto-keep, once it has learned from you (default 0.80)")
    p.add_argument("--sure-reject", type=float, default=0.08,
                   help="Auto-reject when the keep chance is this low or lower (default 0.08)")
    args = p.parse_args()
    ds = Dataset(Path(args.root), args.ai_model)
    if args.apply:
        print(ds.auto_sort(args))
        print(ds.apply())
        return 0
    if not args.skip_checks:
        ds.run_checks(args.workers)
    print(ds.auto_sort(args))
    print(ds.learn_info)
    ds.ai_worker = AIChecker(ds, args)
    todo = ds.ai_todo()
    for uid in todo:
        ds.ai_worker.add(uid)
    left = len(ds.thumbs_todo())
    if left:
        print(f"{left} models need pictures: the review page makes them (keep it open in front).")
    if todo and ds.ai_worker.enabled:
        print(f"AI check: {len(todo)} models waiting.")
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(ds, args))
    url = f"http://127.0.0.1:{args.port}/"
    print(f"\nReview page: {url}  (leave this window open while you review; close it when done)")
    webbrowser.open(url)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
