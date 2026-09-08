#!/usr/bin/env python3
"""Create a source ZIP and SHA-256 inventory, excluding known build outputs/env files."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import stat
import zipfile

EXCLUDED_PARTS = {".git", "target", "__pycache__", ".venv"}
AGENT_TRANSIENT = {".cache", "runtime", "tmp"}


def eligible(path: Path, root: Path) -> bool:
    relative = path.relative_to(root)
    if EXCLUDED_PARTS.intersection(relative.parts) or path.is_symlink() or not path.is_file():
        return False
    if len(relative.parts) > 1 and relative.parts[0] == ".agent":
        if relative.parts[1] in AGENT_TRANSIENT or relative.as_posix() == ".agent/state/adopt-last.json":
            return False
    if "validation/local" in relative.as_posix():
        return False
    if path.name.startswith(".env") or path.suffix in {".log", ".zip", ".pyc"}:
        return False
    allowed = {".rs", ".md", ".toml", ".py", ".sh", ".yml", ".yaml", ".json", ".jsonl"}
    named = {"LICENSE", "Cargo.lock", ".gitignore", ".gitattributes", ".gitkeep"}
    if relative.as_posix() == "scripts/jig":
        return True
    return path.name != "SHA256SUMS" and (path.suffix in allowed or path.name in named)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[1]
    parser.add_argument("--output", type=Path, default=root.parent / "batter-0.1.0-mvp.zip")
    args = parser.parse_args()
    paths = sorted(path for path in root.rglob("*") if eligible(path, root))
    inventory = root / "SHA256SUMS"
    inventory.write_text("".join(f"{digest(path)}  {path.relative_to(root).as_posix()}\n" for path in paths))
    paths.append(inventory)
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(paths):
            member = "batter/" + path.relative_to(root).as_posix()
            info = zipfile.ZipInfo(member, date_time=(2026, 9, 7, 0, 0, 0))
            info.create_system = 3
            info.compress_type = zipfile.ZIP_DEFLATED
            mode = 0o755 if path.parent.name == "scripts" else 0o644
            info.external_attr = (stat.S_IFREG | mode) << 16
            archive.writestr(info, path.read_bytes())
    with zipfile.ZipFile(output) as archive:
        bad = archive.testzip()
        if bad:
            raise RuntimeError(f"ZIP CRC failed: {bad}")
    checksum = output.with_suffix(output.suffix + ".sha256")
    checksum.write_text(f"{digest(output)}  {output.name}\n")
    print(f"Created {output.name}: {len(paths)} source/documentation files, {output.stat().st_size} bytes.")
    print(f"SHA-256: {digest(output)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
