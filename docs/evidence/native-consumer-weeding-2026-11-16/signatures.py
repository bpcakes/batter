"""Compare public function signatures in runtime rustdoc JSON snapshots.
Uses the same local module/re-export reachability as the checked-in surface.py.
Numeric rustdoc IDs are stripped; output is manually checked against source.
"""
import json
import sys
from pathlib import Path

def normalize(value):
    if isinstance(value, dict):
        return {k: normalize(v) for k, v in value.items() if k != "id"}
    if isinstance(value, list):
        return [normalize(v) for v in value]
    return value

def functions(doc):
    index = doc["index"]
    result = {}
    def record(item, path):
        fn = item["inner"]["function"]
        result["::".join(path)] = normalize({key: fn[key] for key in ("sig", "generics", "header")})
    def walk(item_id, prefix):
        item = index.get(str(item_id))
        if item is None or item.get("crate_id", 0) != 0:
            return
        inner = item.get("inner", {})
        kind = next(iter(inner))
        name = item.get("name")
        visibility = item.get("visibility")
        if kind == "module":
            if visibility not in ("public", "default") and str(item_id) != str(doc["root"]):
                return
            path = prefix if name is None else prefix + [name]
            for child in inner["module"]["items"]:
                walk(child, path)
            return
        if visibility not in ("public", "default"):
            return
        if kind == "use":
            target = inner["use"].get("id")
            target_item = index.get(str(target), {})
            if target is not None and target_item.get("crate_id", 0) == 0 and "module" not in target_item.get("inner", {}):
                walk(target, prefix)
            return
        if name is None:
            return
        path = prefix + [name]
        if kind == "function":
            record(item, path)
        if kind in ("struct", "enum", "union", "trait", "type_alias", "primitive"):
            for impl_id in inner[kind].get("impls", []):
                impl = index[str(impl_id)]["inner"]["impl"]
                if impl.get("trait") is not None:
                    continue
                for member in impl.get("items", []):
                    method = index.get(str(member), {})
                    if method.get("visibility") in ("public", "default") and method.get("name") and "function" in method.get("inner", {}):
                        record(method, path + [method["name"]])
            if kind == "trait":
                for member in inner["trait"].get("items", []):
                    method = index.get(str(member), {})
                    if method.get("name") and "function" in method.get("inner", {}):
                        record(method, path + [method["name"]])
    walk(doc["root"], [])
    return result

if len(sys.argv) != 3:
    raise SystemExit("usage: signatures.py <before-runtime.json> <after-runtime.json>")
old = functions(json.loads(Path(sys.argv[1]).read_text()))
new = functions(json.loads(Path(sys.argv[2]).read_text()))
changed = {path: {"before": old[path], "after": new[path]} for path in sorted(old.keys() & new.keys()) if old[path] != new[path]}
print(json.dumps({"changed_paths": len(changed), "changes": changed}, indent=2))
