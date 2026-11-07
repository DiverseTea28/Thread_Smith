import unreal
out = []
sub = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
for a in sub.get_all_level_actors():
    comps = a.get_components_by_class(unreal.SkeletalMeshComponent)
    meshes = [(c.get_name(), c.get_skeletal_mesh_asset().get_path_name() if c.get_skeletal_mesh_asset() else None) for c in comps]
    if any(m and ("Body" in m or "MetaHuman" in m) for _, m in meshes):
        out.append("%s | %s | %s" % (a.get_actor_label(), a.get_class().get_name(), meshes[:6]))
ar = unreal.AssetRegistryHelpers.get_asset_registry()
bps = ar.get_assets_by_path("/Game/MetaHumans", recursive=True)
out.append("--- MetaHuman blueprints:")
out += [str(b.package_name) for b in bps if str(b.asset_name).startswith("BP_")]
world = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem).get_editor_world()
out.append("--- level: " + world.get_path_name())
open(r"C:/Users/tyco0/Documents/ThreadSmith/unreal/find_mh.txt", "w").write("\n".join(out))
