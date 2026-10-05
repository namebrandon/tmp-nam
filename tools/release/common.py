"""Shared helpers for the maintainer release tools (Python 3.11+, stdlib only)."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RELEASE = ROOT / "device" / "release.json"
DEVICE = ROOT / "device"
BINARIES = {  # release.json asset key -> file under device/bin
    "nam_dispatch": "nam_dispatch.so",
    "dropbear": "dropbear",
    "dropbearkey": "dropbearkey",
}


def digest_bytes(data):
    return hashlib.sha256(data).hexdigest()


def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def load_release():
    return json.loads(RELEASE.read_text(encoding="utf-8"))


def compiled_inputs(root=ROOT):
    """{path: sha256} for exactly the player's compiled inputs (see compiled_inputs.json)."""
    spec = json.loads((ROOT / "tools/release/compiled_inputs.json").read_text(encoding="utf-8"))
    names = set(spec["files"])
    for pattern in spec["patterns"]:
        names.update(str(p.relative_to(root)) for p in root.glob(pattern))
    records = {}
    for name in sorted(names):
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise RuntimeError("Required compiled input is missing or not a regular file: " + name)
        records[name] = digest(path)
    return records
