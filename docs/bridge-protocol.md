# ThreadSmith ↔ Unreal bridge protocol (version 1)

ThreadSmith and the **ThreadSmith Bridge** Unreal plugin talk over plain HTTP
with JSON bodies, on this PC only (`127.0.0.1`). Nothing goes over the network.

| Side | Listens on | Receives |
| --- | --- | --- |
| ThreadSmith app | `127.0.0.1:47830` | bodies from Unreal |
| Unreal plugin | `127.0.0.1:47831` | outfits from ThreadSmith |

Each side checks the other with `GET /status` (ThreadSmith does this every 3
seconds and shows "Unreal connected" in its status bar).

## ThreadSmith app (port 47830)

### `GET /status`

```json
{ "ok": true, "app": "ThreadSmith", "version": "0.1.0", "protocol": 1 }
```

### `POST /body` — open a body in ThreadSmith

The plugin exports the MetaHuman body skeletal mesh to FBX first, then sends:

```json
{
  "file": "C:/Users/you/AppData/Local/Temp/ThreadSmith/Ryan_Body.fbx",
  "character": "Ryan",
  "skeleton_asset": "/Game/MetaHumans/Common/Female/Medium/NormalWeight/Body/metahuman_base_skel",
  "body_asset": "/Game/MetaHumans/MainCharacter_FullBody",
  "head_file": "C:/.../Saved/ThreadSmith/Bodies/Ryan_Head.fbx"
}
```

`head_file` is optional: the character's face mesh, exported separately.
ThreadSmith adds it to the body (skin weights moved onto the body skeleton by
bone name; face bones go to `head`). It is shown and posed, but clothes are
not fitted to it.

Answer: `{ "ok": true }` (the file is loaded in the background), or
`{ "ok": false, "error": "..." }` with status 400 / 404.

`skeleton_asset` matters: outfits sent back are imported onto this skeleton.
Send the skeleton that actually loads in the project (see the note at the end).

## Unreal plugin (port 47831)

### `GET /status`

```json
{ "ok": true, "plugin": "ThreadSmithBridge", "engine": "5.8.3", "protocol": 1 }
```

### `POST /outfit` — import an outfit

```json
{
  "outfit_dir": "C:/Users/you/Documents/ThreadSmith/Library/Outfits/tshirt-766b5e",
  "mesh_file": "C:/Users/you/Documents/ThreadSmith/Library/Outfits/tshirt-766b5e/outfit.glb",
  "name": "TShirt",
  "category": "Top",
  "character": "Ryan",
  "skeleton_asset": "/Game/MetaHumans/Common/Female/Medium/NormalWeight/Body/metahuman_base_skel",
  "body_asset": "/Game/MetaHumans/MainCharacter_FullBody",
  "apply_to_character": true
}
```

The plugin should:

1. Import `mesh_file` with Interchange into `/Game/ThreadSmith/Outfits/<name>/`,
   with the generic assets pipeline's
   `CommonSkeletalMeshesAndAnimationsProperties.Skeleton` set to
   `skeleton_asset`, static meshes and animations off. (Tested in UE 5.8.3:
   this reuses the MetaHuman skeleton and creates no new skeleton.)
2. If `apply_to_character` is true: add the skeletal mesh to the character
   (selected MetaHuman actor / its Blueprint) so it follows the body.
3. Answer `{ "ok": true, "asset": "/Game/ThreadSmith/Outfits/TShirt/TShirt" }`
   or `{ "ok": false, "error": "..." }`.

## Outfit library (for the wardrobe)

The wardrobe reads the library folder directly, so it works when ThreadSmith is
closed. Default location: `Documents/ThreadSmith/Library`.

```text
Library/
  Outfits/
    <slug>-<6 hex>/
      outfit.json     info (below)
      outfit.glb      skinned mesh(es), Y up, meters, glTF 2.0, up to 8 bone weights
      thumbnail.png   512 x 512
```

`outfit.json`:

```json
{
  "format": 1,
  "id": "766b5e0bf7d7cb64",
  "name": "TShirt",
  "category": "Top",
  "created": 1793831900,
  "modified": 1793831900,
  "garments": [{ "name": "TShirt", "triangles": 7854 }],
  "body": {
    "character": "Ryan",
    "bones": 342,
    "unreal_skeleton": "/Game/MetaHumans/Common/Female/Medium/NormalWeight/Body/metahuman_base_skel",
    "unreal_body": "/Game/MetaHumans/MainCharacter_FullBody"
  },
  "mesh_file": "outfit.glb",
  "thumbnail_file": "thumbnail.png",
  "tags": []
}
```

Categories: `Top`, `Bottom`, `Full body`, `Footwear`, `Headwear`, `Gloves`,
`Accessory`, `Outfit`. Readers should ignore unknown fields and skip folders
whose `format` is newer than they understand.

## Note: MetaHuman skeletons in UE 5.8

In Tyler's project, `MainCharacter_FullBody` points at
`/MetaHumanCharacter/.../metahuman_base_skel` (inside the MetaHuman Character
plugin), which isn't mounted, so the mesh's skeleton loads as empty. A copy
lives at `/Game/MetaHumans/Common/Female/Medium/NormalWeight/Body/metahuman_base_skel`.
The plugin should use the body's skeleton when it loads, and otherwise fall
back to a skeleton named `metahuman_base_skel` under `/Game`.
