# Copies the freshly built ThreadSmith Bridge into the open project's Plugins folder.
# Files Unreal has locked (the loaded DLLs) are renamed to *.old first; Windows allows that.
# Restart Unreal afterwards to load the new version.
import os, shutil, unreal
src = r'C:\Users\tyco0\Documents\ThreadSmith\unreal\Build\ThreadSmithBridge'
dst = os.path.join(unreal.Paths.convert_relative_path_to_full(unreal.Paths.project_plugins_dir()), 'ThreadSmithBridge')
log = []
for root, dirs, files in os.walk(src):
    rel = os.path.relpath(root, src)
    if rel.split(os.sep)[0] in ('HostProject', 'Intermediate'):
        continue
    os.makedirs(os.path.join(dst, rel), exist_ok=True)
    for f in files:
        s, d = os.path.join(root, f), os.path.join(dst, rel, f)
        try:
            shutil.copy2(s, d)
        except PermissionError:
            old = d + '.old'
            if os.path.exists(old):
                os.remove(old)
            os.replace(d, old)
            shutil.copy2(s, d)
            log.append('swapped ' + os.path.join(rel, f))
log.append('installed to ' + dst)
open(r'C:\Users\tyco0\Documents\ThreadSmith\unreal\install_log.txt', 'w').write('\n'.join(log))
