"""Extract a public API surface from rustdoc JSON and diff two revisions.

Usage: surface.py <old-doc-dir> <new-doc-dir> <crate-name>...
Prints removed and added entries per crate. Entries are kind-qualified paths:
modules, structs, enums, variants, traits, trait items, functions, type aliases,
constants, inherent impl methods, and `use` re-exports by source path.
"""
import json
import sys
from pathlib import Path


def load(doc_dir: Path, crate: str):
    path = doc_dir / f"{crate.replace('-', '_')}.json"
    if not path.exists():
        return None
    return json.loads(path.read_text())


def item_kind(item):
    inner = item.get("inner", {})
    if isinstance(inner, dict) and inner:
        return next(iter(inner))
    return str(inner)


def surface(doc):
    index = doc["index"]
    root = str(doc["root"])
    entries = set()

    def walk(item_id, prefix):
        item = index.get(str(item_id))
        if item is None or item.get("crate_id", 0) != 0:
            return
        name = item.get("name")
        kind = item_kind(item)
        inner = item.get("inner", {}) or {}
        vis = item.get("visibility")
        if kind == "module":
            path = prefix if name is None else prefix + [name]
            if vis not in ("public", "default") and item_id != root:
                return
            if name is not None:
                entries.add(("mod", "::".join(path)))
            for child in inner["module"]["items"]:
                walk(child, path)
            return
        if vis not in ("public", "default"):
            return
        if kind == "use":
            use = inner["use"]
            glob = "::*" if use.get("is_glob") else ""
            entries.add(("use", "::".join(prefix + [use["name"]]) + " <- " + use["source"] + glob))
            # Follow inlined local re-exports so facade modules list their contents.
            target = use.get("id")
            if target is not None and str(target) in index and index[str(target)].get("crate_id", 0) == 0:
                # Avoid cycles on glob re-exports of ancestors.
                tgt = index[str(target)]
                if item_kind(tgt) != "module":
                    walk(target, prefix)
            return
        if name is None:
            return
        path = prefix + [name]
        entries.add((kind, "::".join(path)))
        if kind in ("struct", "enum", "union", "trait", "type_alias", "primitive"):
            for impl_id in inner.get(kind, {}).get("impls", []):
                impl_item = index.get(str(impl_id))
                if impl_item is None:
                    continue
                impl_inner = impl_item["inner"]["impl"]
                trait = impl_inner.get("trait")
                if trait is not None:
                    if impl_inner.get("is_synthetic") or impl_inner.get("blanket_impl"):
                        continue
                    entries.add(("impl", "::".join(path) + ": " + trait["path"]))
                    continue
                for member in impl_inner.get("items", []):
                    m = index.get(str(member))
                    if m is None or m.get("name") is None:
                        continue
                    if m.get("visibility") not in ("public", "default"):
                        continue
                    entries.add(("method", "::".join(path) + "::" + m["name"]))
            if kind == "enum":
                for variant in inner["enum"]["variants"]:
                    v = index.get(str(variant))
                    if v is not None and v.get("name"):
                        entries.add(("variant", "::".join(path) + "::" + v["name"]))
            if kind == "struct":
                sk = inner["struct"]["kind"]
                if isinstance(sk, dict) and "plain" in sk:
                    for field in sk["plain"]["fields"]:
                        f = index.get(str(field))
                        if f is not None and f.get("visibility") == "public" and f.get("name"):
                            entries.add(("field", "::".join(path) + "." + f["name"]))
            if kind == "trait":
                for member in inner["trait"]["items"]:
                    m = index.get(str(member))
                    if m is not None and m.get("name"):
                        entries.add(("trait_item", "::".join(path) + "::" + m["name"]))

    walk(root, [])
    return entries


def main():
    old_dir, new_dir, *crates = sys.argv[1:]
    for crate in crates:
        old = load(Path(old_dir), crate)
        new = load(Path(new_dir), crate)
        if old is None or new is None:
            print(f"## {crate}: present only in {'new' if old is None else 'old'}")
            continue
        o, n = surface(old), surface(new)
        removed = sorted(o - n)
        added = sorted(n - o)
        print(f"## {crate}: {len(o)} -> {len(n)} entries; {len(removed)} removed, {len(added)} added")
        for kind, path in removed:
            print(f"- REMOVED {kind} {path}")
        for kind, path in added:
            print(f"+ ADDED   {kind} {path}")
        print()


if __name__ == "__main__":
    main()
