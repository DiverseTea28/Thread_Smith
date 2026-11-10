"""Collect clothing 3D models that can legally be used to train ThreadSmith's
clothing LoRA, into "Training data", sorted by garment type and renamed.

Run it with "Collect clothing.bat" (in the Training data folder), or:
    python collect_clothing.py "C:\\Users\\you\\Documents\\ThreadSmith\\Training data"

Only the Python standard library is needed (ComfyUI's Python works).

Sources
1. Objaverse 1.0 (allenai/objaverse on Hugging Face): 800K Sketchfab models
   with their licence in the metadata. The model files are downloaded from
   Hugging Face.
2. Sketchfab search (optional, newer models): only if a Sketchfab API token
   is saved in "Training data\\sketchfab_token.txt" (sketchfab.com > Settings >
   Password & API).

Rules (a model is kept only if ALL are true; anything unsure is left out)
- Licence is CC0 or CC-BY (CC-BY: the credits go in CREDITS.md).
  Left out: NC (non-commercial), ND (no changes), SA (share-alike), Standard,
  Editorial, unknown.
- The model's *current* Sketchfab page still exists, is still CC0/CC-BY,
  and has no "NoAI" tag (Sketchfab's no-AI-training tag).
- Not tagged "CreatedWithAI" (made by AI).
- The description and tags say nothing against AI use ("no AI", "not for AI
  training", "do not use in datasets"...). Any mention of AI or machine
  learning that isn't clearly a permission also leaves the model out.
- The name/tags clearly say it's a garment of one of ThreadSmith's types, and
  nothing says it's a character, scene, pack, toy, etc.

Output (inside Training data)
    <Type>\\<type>_<name>_<id8>.glb     the model
    <Type>\\<type>_<name>_<id8>.json    where it's from, author, licence, checks
    _check\\<Type>\\...                  rigged/animated files (may hold a body); look at these by hand
    CREDITS.md, credits.csv             credits for every kept model
    excluded.csv                        every left-out model and why
    collect_log.txt                     what happened
    _state\\                             progress (the run can be stopped and started again)
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import csv
import gzip
import json
import os
import re
import shutil
import struct
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

# (The environment overrides are only for testing against a local copy.)
HF_BASE = os.environ.get("TS_HF_BASE", "https://huggingface.co/datasets/allenai/objaverse/resolve/main")
SKETCHFAB_API = os.environ.get("TS_SKETCHFAB_API", "https://api.sketchfab.com/v3")
USER_AGENT = "ThreadSmith-dataset-builder/1.0 (open-source clothing tool)"
SHARDS = int(os.environ.get("TS_SHARDS", "160"))

# ---------------------------------------------------------------------------
# Garment types (ThreadSmith's presets) and the words that pick them
# ---------------------------------------------------------------------------

TYPES: dict[str, list[str]] = {
    "Jackets": ["jacket", "coat", "parka", "blazer", "windbreaker", "bomber jacket", "cardigan", "raincoat",
                "trench coat", "overcoat", "anorak", "puffer"],
    "Vests": ["vest", "waistcoat", "gilet", "tactical vest", "life vest", "puffer vest"],
    "Shirts": ["shirt", "t-shirt", "tshirt", "t shirt", "tee", "blouse", "polo", "hoodie", "sweater", "sweatshirt",
               "jumper", "tank top", "crop top", "jersey", "pullover", "turtleneck", "tunic", "camisole"],
    "Dresses": ["dress", "gown", "sundress", "kimono", "sari", "qipao", "cheongsam"],
    "Full Body": ["jumpsuit", "overalls", "coverall", "coveralls", "onesie", "bodysuit", "wetsuit", "romper",
                  "dungarees", "boiler suit", "hazmat suit", "flight suit"],
    "Pants": ["pants", "trousers", "jeans", "shorts", "leggings", "joggers", "sweatpants", "cargo pants", "chinos",
              "slacks", "breeches"],
    "Skirts": ["skirt", "miniskirt", "kilt"],
    "Shoes": ["shoe", "shoes", "sneaker", "sneakers", "boot", "boots", "sandal", "sandals", "high heels", "heels",
              "loafer", "loafers", "slipper", "slippers", "trainers", "footwear", "flip flops", "clogs", "moccasin",
              "oxfords", "stiletto"],
    "Gloves": ["glove", "gloves", "mitten", "mittens", "gauntlet", "gauntlets"],
    "Head": ["hat", "cap", "beanie", "helmet", "beret", "fedora", "bandana", "balaclava", "headband", "bucket hat",
             "baseball cap", "cowboy hat", "top hat", "sombrero", "hood", "headwear", "bonnet", "turban", "visor"],
    "Belts": ["belt", "belts", "utility belt"],
    "Backpacks": ["backpack", "rucksack", "knapsack", "school bag", "hiking bag"],
    "Accessory": ["scarf", "necktie", "bow tie", "sunglasses", "eyeglasses", "glasses", "necklace", "bracelet",
                  "earring", "earrings", "handbag", "purse", "wristwatch", "watch", "choker", "suspenders"],
}

# Words that mean "not a single wearable garment".
EXCLUDE = [
    "character", "avatar", "girl", "boy", "woman", "man", "lady", "guy", "person", "people", "human", "npc",
    "body", "mannequin", "doll", "figure", "figurine", "statue", "toy", "plush", "lego", "minifig",
    "scene", "room", "diorama", "store", "shop", "showroom", "wardrobe", "closet", "rack", "hanger", "display",
    "shelf", "pack", "bundle", "collection", "set", "outfit", "kit", "asset pack", "props",
    "soldier", "warrior", "knight", "robot", "mech", "creature", "monster", "animal", "dog", "cat",
    "car", "vehicle", "truck", "bike", "motorcycle", "plane", "ship", "building", "house", "furniture",
    "bottle", "wine", "cake", "food", "pizza", "lamp", "logo", "icon", "sign", "poster", "texture", "material",
    "keyboard", "phone", "chess", "watchtower", "lighthouse", "test", "wip", "sculpt", "base mesh",
    "nsfw", "lingerie", "underwear", "bra", "bikini", "swimsuit",
]

# Words that are fine with a garment word ("shoe rack" is excluded, "shoe" is fine).
ALLOWED_LICENCES = {"cc0", "by"}

AI_WORDS = r"(?:\bai\b|a\.i\.|artificial intelligence|machine learning|\bml\b|neural|dataset|data set|data-set|" \
           r"generative|training data|\bllm\b|diffusion|midjourney|stable diffusion|chatgpt)"
NO_AI_TAGS = {"noai", "no-ai", "no_ai", "notforai", "not-for-ai", "noaitraining", "no-ai-training", "donotuseforai",
              "createdwithai", "created-with-ai", "aigenerated", "ai-generated", "madewithai", "made-with-ai",
              "generated-with-ai", "aigen", "ai-made"}


def norm_licence(value) -> str:
    """Objaverse / Sketchfab licence -> 'cc0', 'by', 'by-sa', ... or ''."""
    if isinstance(value, dict):
        value = value.get("slug") or value.get("label") or ""
    v = str(value or "").strip().lower()
    v = v.replace("creative commons", "").replace("attribution", "by").replace("public domain", "").strip()
    v = v.removeprefix("cc-").removeprefix("cc ").strip()
    table = {
        "cc0": "cc0", "0": "cc0", "cc0 1.0": "cc0", "cc0 public domain": "cc0",
        "by": "by", "by 4.0": "by", "by-4.0": "by",
        "by-sa": "by-sa", "by-nc": "by-nc", "by-nd": "by-nd", "by-nc-sa": "by-nc-sa", "by-nc-nd": "by-nc-nd",
        "st": "standard", "standard": "standard", "ed": "editorial", "editorial": "editorial",
    }
    return table.get(v, v)


def words(text: str) -> str:
    return " " + re.sub(r"[^a-z0-9]+", " ", (text or "").lower()).strip() + " "


def has_word(haystack: str, word: str) -> bool:
    return f" {words(word).strip()} " in haystack


def classify(name: str, tags: list[str]) -> tuple[str | None, str]:
    """(garment type, reason). Needs a garment word in the name, or several
    clear tag matches with a fashion category; and no exclude word."""
    n = words(name)
    t = words(" ".join(tags))
    for w in EXCLUDE:
        if has_word(n, w):
            return None, f"name says '{w}'"
    best, best_score = None, 0
    for typ, ws in TYPES.items():
        score = 0
        for w in ws:
            if has_word(n, w):
                score += 3
            if has_word(t, w):
                score += 1
        if score > best_score:
            best, best_score = typ, score
    if best is None:
        return None, "no garment word"
    if best_score < 3:
        return None, "garment word only in tags"
    return best, ""


def ai_problem(description: str, tags: list[str]) -> str:
    """Reason to leave the model out because of AI wording, or ''."""
    slugs = {re.sub(r"[^a-z0-9-]", "", s.lower().replace(" ", "-")) for s in tags}
    flat = {s.replace("-", "") for s in slugs}
    for s in slugs | flat:
        if s in NO_AI_TAGS or s.replace("-", "") in {x.replace("-", "") for x in NO_AI_TAGS}:
            return f"tag '{s}'"
    text = (description or "").lower()
    if re.search(r"\bno\s*-?\s*ai\b|\bnoai\b", text):
        return "description says no AI"
    if re.search(AI_WORDS, text):
        # Any mention of AI / datasets / training: unsure, so leave it out.
        m = re.search(AI_WORDS, text)
        start = max(0, m.start() - 40)
        return "description mentions AI: …" + text[start:m.end() + 40].replace("\n", " ") + "…"
    return ""


def slug(text: str, n: int = 40) -> str:
    s = re.sub(r"[^a-z0-9]+", "_", (text or "").lower()).strip("_")
    return (s[:n].rstrip("_")) or "model"


# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------


class Log:
    def __init__(self, path: Path):
        self.f = open(path, "a", encoding="utf-8")
        self.lock = threading.Lock()

    def __call__(self, text: str) -> None:
        line = f"[{time.strftime('%H:%M:%S')}] {text}"
        with self.lock:
            print(line, flush=True)
            self.f.write(line + "\n")
            self.f.flush()


def http_get(url: str, headers: dict | None = None, timeout: int = 60, tries: int = 5) -> bytes:
    h = {"User-Agent": USER_AGENT}
    h.update(headers or {})
    delay = 2.0
    for attempt in range(tries):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=h), timeout=timeout) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503, 504) and attempt < tries - 1:
                wait = float(e.headers.get("Retry-After") or delay)
                time.sleep(min(wait, 120))
                delay *= 2
                continue
            raise
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            if attempt < tries - 1:
                time.sleep(delay)
                delay *= 2
                continue
            raise
    raise RuntimeError("unreachable")


def download_file(url: str, dest: Path, max_bytes: int, headers: dict | None = None) -> int:
    """Download to dest (via .part). Returns size. Raises ValueError if too big."""
    h = {"User-Agent": USER_AGENT}
    h.update(headers or {})
    part = dest.with_suffix(dest.suffix + ".part")
    req = urllib.request.Request(url, headers=h)
    with urllib.request.urlopen(req, timeout=120) as r:
        total = int(r.headers.get("Content-Length") or 0)
        if total and total > max_bytes:
            raise ValueError(f"too big ({total / 1e6:.0f} MB)")
        done = 0
        with open(part, "wb") as f:
            while True:
                chunk = r.read(1 << 20)
                if not chunk:
                    break
                done += len(chunk)
                if done > max_bytes:
                    f.close()
                    part.unlink(missing_ok=True)
                    raise ValueError(f"too big (> {max_bytes / 1e6:.0f} MB)")
                f.write(chunk)
    os.replace(part, dest)
    return done


def glb_info(path: Path) -> dict:
    """Peek inside a GLB: skins (skeleton), animations, meshes, textures."""
    try:
        with open(path, "rb") as f:
            magic, _version, _length = struct.unpack("<III", f.read(12))
            if magic != 0x46546C67:
                return {"valid": False}
            chunk_len, chunk_type = struct.unpack("<II", f.read(8))
            if chunk_type != 0x4E4F534A:
                return {"valid": False}
            js = json.loads(f.read(chunk_len))
        return {
            "valid": True,
            "skins": len(js.get("skins", [])),
            "animations": len(js.get("animations", [])),
            "meshes": len(js.get("meshes", [])),
            "nodes": len(js.get("nodes", [])),
            "textures": len(js.get("images", [])),
        }
    except Exception as exc:  # noqa: BLE001
        return {"valid": False, "error": str(exc)}


# ---------------------------------------------------------------------------
# Main work
# ---------------------------------------------------------------------------


class Collector:
    def __init__(self, out: Path, args):
        self.out = out
        self.args = args
        self.state_dir = out / "_state"
        self.state_dir.mkdir(parents=True, exist_ok=True)
        self.log = Log(out / "collect_log.txt")
        self.lock = threading.Lock()
        self.done = self._load_jsonl("done.jsonl")          # uid -> record (kept)
        self.excluded = self._load_jsonl("excluded.jsonl")  # uid -> {reason...}
        self.token = None
        tok = out / "sketchfab_token.txt"
        if tok.is_file():
            self.token = tok.read_text(encoding="utf-8").strip() or None
        self.bytes_total = sum(r.get("bytes", 0) for r in self.done.values())
        self.counts: dict[str, int] = {}
        for r in self.done.values():
            self.counts[r["type"]] = self.counts.get(r["type"], 0) + 1

    # ---- state ----
    def _load_jsonl(self, name: str) -> dict:
        p = self.state_dir / name
        out = {}
        if p.is_file():
            for line in p.read_text(encoding="utf-8").splitlines():
                try:
                    r = json.loads(line)
                    out[r["uid"]] = r
                except Exception:
                    pass
        return out

    def _append(self, name: str, rec: dict) -> None:
        with self.lock, open(self.state_dir / name, "a", encoding="utf-8") as f:
            f.write(json.dumps(rec) + "\n")

    def exclude(self, uid: str, name: str, source: str, reason: str, url: str = "") -> None:
        rec = {"uid": uid, "name": name, "source": source, "reason": reason, "url": url}
        with self.lock:
            self.excluded[uid] = rec
        self._append("excluded.jsonl", rec)

    def seen(self, uid: str) -> bool:
        return uid in self.done or uid in self.excluded

    # ---- step 1: Objaverse metadata ----
    def objaverse_candidates(self) -> list[dict]:
        cache = self.state_dir / "objaverse_candidates.json"
        if cache.is_file():
            cands = json.loads(cache.read_text(encoding="utf-8"))
            self.log(f"Objaverse: {len(cands)} candidates (from the saved list)")
            return cands
        meta_dir = self.state_dir / "objaverse_metadata"
        meta_dir.mkdir(exist_ok=True)
        cands = []
        stats = {"total": 0, "licence": 0, "not_garment": 0}
        for i in range(SHARDS):
            shard = f"000-{i:03d}"
            path = meta_dir / f"{shard}.json.gz"
            if not path.is_file():
                self.log(f"Objaverse: reading metadata {i + 1}/{SHARDS}")
                data = http_get(f"{HF_BASE}/metadata/{shard}.json.gz", timeout=300)
                path.write_bytes(data)
            meta = json.loads(gzip.decompress(path.read_bytes()))
            for uid, m in meta.items():
                stats["total"] += 1
                lic = norm_licence(m.get("license"))
                if lic not in ALLOWED_LICENCES:
                    stats["licence"] += 1
                    continue
                tags = [t.get("name", "") if isinstance(t, dict) else str(t) for t in m.get("tags", [])]
                typ, why = classify(m.get("name", ""), tags)
                if typ is None:
                    stats["not_garment"] += 1
                    continue
                cands.append({
                    "uid": uid, "shard": shard, "name": m.get("name", ""), "type": typ, "licence": lic,
                    "faces": m.get("faceCount") or 0, "tags": tags,
                    "description": (m.get("description") or "")[:4000],
                    "author": (m.get("user") or {}).get("displayName") or (m.get("user") or {}).get("username", ""),
                    "author_url": (m.get("user") or {}).get("profileUrl", ""),
                    "url": m.get("viewerUrl") or f"https://sketchfab.com/3d-models/{uid}",
                })
        cache.write_text(json.dumps(cands), encoding="utf-8")
        self.log(
            f"Objaverse: {stats['total']} models read, {stats['licence']} left out for licence, "
            f"{stats['not_garment']} not a garment, {len(cands)} garment candidates"
        )
        if not self.args.keep_metadata:
            shutil.rmtree(meta_dir, ignore_errors=True)
        return cands

    # ---- step 2: live check on Sketchfab ----
    def live_check(self, uid: str) -> tuple[dict | None, str]:
        """Current Sketchfab page data, or (None, reason)."""
        try:
            data = json.loads(http_get(f"{SKETCHFAB_API}/models/{uid}", timeout=30))
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None, "no longer on Sketchfab (can't check its licence today)"
            return None, f"Sketchfab check failed ({e.code})"
        except Exception as exc:  # noqa: BLE001
            return None, f"Sketchfab check failed ({type(exc).__name__})"
        lic = norm_licence(data.get("license"))
        if lic not in ALLOWED_LICENCES:
            return None, f"licence today is '{lic or 'unknown'}'"
        tags = [t.get("name", "") if isinstance(t, dict) else str(t) for t in data.get("tags", [])]
        tags += [t.get("slug", "") for t in data.get("tags", []) if isinstance(t, dict)]
        problem = ai_problem(data.get("description", ""), tags)
        if problem:
            return None, "AI: " + problem
        if data.get("isAgeRestricted"):
            return None, "age restricted"
        return data, ""

    # ---- step 3: download one ----
    def fetch(self, c: dict) -> None:
        uid = c["uid"]
        if self.seen(uid):
            return
        if self.args.max_per_type and self.counts.get(c["type"], 0) >= self.args.max_per_type:
            return
        if self.args.max_gb and self.bytes_total > self.args.max_gb * 1e9:
            return
        if c.get("faces") and (c["faces"] > self.args.max_faces or c["faces"] < 50):
            self.exclude(uid, c["name"], c["source"], f"{c['faces']} faces", c["url"])
            return
        # Objaverse's metadata says nothing against AI either.
        problem = ai_problem(c.get("description", ""), c.get("tags", []))
        if problem:
            self.exclude(uid, c["name"], c["source"], "AI: " + problem, c["url"])
            return
        live, why = self.live_check(uid)
        if live is None:
            self.exclude(uid, c["name"], c["source"], why, c["url"])
            return
        # Re-classify with today's name and tags.
        live_tags = [t.get("name", "") for t in live.get("tags", []) if isinstance(t, dict)]
        typ, why = classify(live.get("name") or c["name"], live_tags)
        if typ is None:
            self.exclude(uid, c["name"], c["source"], why, c["url"])
            return
        name = live.get("name") or c["name"]
        base = f"{slug(typ, 12)}_{slug(name)}_{uid[:8]}"
        folder = self.out / typ
        folder.mkdir(parents=True, exist_ok=True)
        glb = folder / f"{base}.glb"
        try:
            if c["source"] == "objaverse":
                url = f"{HF_BASE}/glbs/{c['shard']}/{uid}.glb"
                size = download_file(url, glb, int(self.args.max_mb * 1e6))
            else:
                info = json.loads(http_get(f"{SKETCHFAB_API}/models/{uid}/download",
                                           headers={"Authorization": f"Token {self.token}"}, timeout=60))
                g = info.get("glb")
                if not g or not g.get("url"):
                    self.exclude(uid, name, c["source"], "no GLB download offered", c["url"])
                    return
                size = download_file(g["url"], glb, int(self.args.max_mb * 1e6))
        except ValueError as exc:
            self.exclude(uid, name, c["source"], str(exc), c["url"])
            return
        except Exception as exc:  # noqa: BLE001
            self.log(f"  download failed, will retry next run: {name} ({type(exc).__name__}: {exc})")
            return
        user = live.get("user") or {}
        author = user.get("displayName") or user.get("username") or c.get("author", "")
        if norm_licence(live.get("license")) == "by" and not author.strip():
            # CC BY needs a credit; without a name we can't give one.
            glb.unlink(missing_ok=True)
            self.exclude(uid, name, c["source"], "CC BY but no author name to credit", c["url"])
            return
        info = glb_info(glb)
        if not info.get("valid"):
            glb.unlink(missing_ok=True)
            self.exclude(uid, name, c["source"], "not a valid GLB", c["url"])
            return
        flag = ""
        if info.get("skins") or info.get("animations"):
            # Rigged or animated: may hold a body. Kept aside for a look.
            flag = "rigged/animated: may include a body, check by hand"
            check = self.out / "_check" / typ
            check.mkdir(parents=True, exist_ok=True)
            glb = Path(shutil.move(str(glb), str(check / glb.name)))
        lic = norm_licence(live.get("license"))
        rec = {
            "uid": uid,
            "type": typ,
            "name": name,
            "file": str(glb.relative_to(self.out)),
            "source": c["source"],
            "url": live.get("viewerUrl") or c["url"],
            "author": author or "unknown",
            "author_url": user.get("profileUrl") or c.get("author_url", ""),
            "licence": "CC0 1.0" if lic == "cc0" else "CC BY 4.0",
            "licence_url": "https://creativecommons.org/publicdomain/zero/1.0/" if lic == "cc0"
            else "https://creativecommons.org/licenses/by/4.0/",
            "ai_check": "no NoAI/CreatedWithAI tag; description says nothing about AI (checked "
                        + time.strftime("%Y-%m-%d") + ")",
            "faces": live.get("faceCount") or c.get("faces"),
            "bytes": size,
            "glb": info,
            "flag": flag,
        }
        glb.with_suffix(".json").write_text(json.dumps(rec, indent=2), encoding="utf-8")
        with self.lock:
            self.done[uid] = rec
            self.bytes_total += size
            self.counts[typ] = self.counts.get(typ, 0) + 1
        self._append("done.jsonl", rec)
        self.log(f"  + {typ}: {name} ({rec['licence']}, {size / 1e6:.1f} MB){'  [check]' if flag else ''}")

    # ---- Sketchfab search (optional) ----
    def sketchfab_candidates(self) -> list[dict]:
        cache = self.state_dir / "sketchfab_candidates.json"
        if cache.is_file():
            return json.loads(cache.read_text(encoding="utf-8"))
        cands: dict[str, dict] = {}
        for typ, ws in TYPES.items():
            for w in ws:
                for lic in ("cc0", "by"):
                    url = (f"{SKETCHFAB_API}/search?type=models&downloadable=true&count=24&license={lic}"
                           f"&q={urllib.parse.quote(w)}")
                    pages = 0
                    while url and pages < self.args.sketchfab_pages:
                        try:
                            data = json.loads(http_get(url, headers={"Authorization": f"Token {self.token}"}))
                        except Exception as exc:  # noqa: BLE001
                            self.log(f"Sketchfab search '{w}' stopped: {exc}")
                            break
                        for m in data.get("results", []):
                            uid = m.get("uid")
                            if not uid or uid in cands or self.seen(uid):
                                continue
                            tags = [t.get("name", "") for t in m.get("tags", []) if isinstance(t, dict)]
                            t2, _ = classify(m.get("name", ""), tags)
                            if t2 is None:
                                continue
                            cands[uid] = {
                                "uid": uid, "name": m.get("name", ""), "type": t2, "licence": lic,
                                "faces": m.get("faceCount") or 0, "tags": tags, "description": "",
                                "url": m.get("viewerUrl") or f"https://sketchfab.com/3d-models/{uid}",
                                "source": "sketchfab",
                            }
                        url = data.get("next")
                        pages += 1
                        time.sleep(1.0)
                self.log(f"Sketchfab search: '{w}' done, {len(cands)} candidates so far")
        out = list(cands.values())
        cache.write_text(json.dumps(out), encoding="utf-8")
        return out

    # ---- credits ----
    def write_reports(self) -> None:
        kept = sorted(self.done.values(), key=lambda r: (r["type"], r["name"].lower()))
        with open(self.out / "credits.csv", "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(["type", "file", "title", "author", "author_url", "licence", "licence_url", "model_url", "flag"])
            for r in kept:
                w.writerow([r["type"], r["file"], r["name"], r["author"], r["author_url"], r["licence"],
                            r["licence_url"], r["url"], r.get("flag", "")])
        lines = [
            "# Credits: ThreadSmith clothing training data",
            "",
            "These 3D models are used to train ThreadSmith's clothing LoRA. Each is under the licence shown.",
            "CC BY 4.0 models: https://creativecommons.org/licenses/by/4.0/ . No changes were made to the",
            "models themselves; they were converted into training data. Sources: Sketchfab, via the Objaverse",
            "dataset (allenai/objaverse, ODC-By 1.0) or the Sketchfab API.",
            "",
        ]
        current = None
        for r in kept:
            if r["type"] != current:
                current = r["type"]
                lines += ["", f"## {current}", ""]
            author = f"[{r['author']}]({r['author_url']})" if r.get("author_url") else r.get("author", "unknown")
            lines.append(f"- \"[{r['name']}]({r['url']})\" by {author}, {r['licence']}")
        (self.out / "CREDITS.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
        with open(self.out / "excluded.csv", "w", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            w.writerow(["name", "reason", "source", "url"])
            for r in self.excluded.values():
                w.writerow([r.get("name", ""), r.get("reason", ""), r.get("source", ""), r.get("url", "")])

    def summary(self) -> str:
        parts = [f"{t}: {n}" for t, n in sorted(self.counts.items())]
        checks = sum(1 for r in self.done.values() if r.get("flag"))
        return (f"Kept {len(self.done)} models ({self.bytes_total / 1e9:.1f} GB; {checks} in _check). "
                f"Left out {len(self.excluded)}. " + ", ".join(parts))

    # ---- run ----
    def run(self) -> None:
        self.log("=" * 60)
        self.log(f"Collecting clothing into {self.out}")
        cands = []
        for c in self.objaverse_candidates():
            c["source"] = "objaverse"
            cands.append(c)
        if self.token:
            self.log("Sketchfab token found: also searching Sketchfab for newer models")
            cands += self.sketchfab_candidates()
        else:
            self.log("No sketchfab_token.txt: skipping the Sketchfab search (Objaverse only)")
        todo = [c for c in cands if not self.seen(c["uid"])]
        if self.args.plan_only:
            by = {}
            for c in todo:
                by[c["type"]] = by.get(c["type"], 0) + 1
            self.log(f"Plan only: {len(todo)} to check: " + ", ".join(f"{k}: {v}" for k, v in sorted(by.items())))
            return
        # Spread types out so a stop part-way still gives some of each.
        todo.sort(key=lambda c: (hash(c["uid"]) % 1000))
        self.log(f"{len(todo)} models to check and download ({len(self.done)} already kept)")
        n = 0
        last_report = time.time()
        pool = cf.ThreadPoolExecutor(max_workers=self.args.workers)
        futures = [pool.submit(self.fetch, c) for c in todo]
        try:
            for fut in cf.as_completed(futures):
                n += 1
                try:
                    fut.result()
                except Exception as exc:  # noqa: BLE001
                    self.log(f"  problem: {type(exc).__name__}: {exc}")
                if time.time() - last_report > 60:
                    last_report = time.time()
                    self.log(f"Progress: {n}/{len(todo)} checked. {self.summary()}")
                    self.write_reports()
        except KeyboardInterrupt:
            pool.shutdown(wait=False, cancel_futures=True)
            raise
        pool.shutdown(wait=True)
        self.write_reports()
        self.log("Finished. " + self.summary())


def main() -> int:
    p = argparse.ArgumentParser(description="Collect legally usable clothing models for ThreadSmith training")
    p.add_argument("out", help="The Training data folder")
    p.add_argument("--workers", type=int, default=3, help="Models checked/downloaded at once (default 3)")
    p.add_argument("--max-mb", type=float, default=150, help="Skip files bigger than this (default 150 MB)")
    p.add_argument("--max-faces", type=int, default=1_500_000, help="Skip models with more faces")
    p.add_argument("--max-gb", type=float, default=200, help="Stop when this much is downloaded (default 200 GB)")
    p.add_argument("--max-per-type", type=int, default=0, help="Stop each type at this many (0 = no limit)")
    p.add_argument("--sketchfab-pages", type=int, default=20, help="Search pages per word (24 models each)")
    p.add_argument("--plan-only", action="store_true", help="Only count what would be checked")
    p.add_argument("--keep-metadata", action="store_true", help="Keep the 580 MB of Objaverse metadata")
    args = p.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    free = shutil.disk_usage(out).free / 1e9
    if free < 20:
        print(f"Only {free:.0f} GB free on this drive. Free some space first.")
        return 1
    args.max_gb = min(args.max_gb, max(free - 15, 1))
    c = Collector(out, args)
    try:
        c.run()
    except KeyboardInterrupt:
        c.write_reports()
        c.log("Stopped. Run again to carry on where it left off. " + c.summary())
    return 0


if __name__ == "__main__":
    sys.exit(main())
