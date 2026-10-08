#!/usr/bin/env python3
"""Publish a verified player build into device/ and device/release.json.

Replaces device/bin/{nam_dispatch.so,dropbear,dropbearkey} and device/licenses,
and rewrites the release pins, atomically with rollback. With --check it writes
nothing and fails unless the build reproduces the checked-in binaries and licenses. It never builds code,
mounts an image, contacts a device, or claims physical boot/audio validation.
The desktop app and `tmp-sdcard` embed release.json at compile time: rebuild them
after publishing, and run `cargo test -p tmp-sdcard` to confirm the pins.
"""
import argparse
import json
import os
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import BINARIES, DEVICE, RELEASE, ROOT, compiled_inputs, digest_bytes, load_release  # noqa: E402

SSH_LICENSES = {"dropbear-LICENSE", "libtomcrypt-LICENSE", "libtommath-LICENSE", "musl-COPYRIGHT", "zig-LICENSE"}


def regular_bytes(path):
    if path.is_symlink() or not path.is_file():
        raise RuntimeError(f"Required regular file is missing: {path}")
    return path.read_bytes()


def validated_licenses(build_dir, provenance):
    licenses = build_dir / "licenses"
    if licenses.is_symlink() or not licenses.is_dir():
        raise RuntimeError("Build licenses directory is missing")
    files = {}
    for path in sorted(licenses.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"License must not be a symlink: {path}")
        if path.is_file():
            files[path.relative_to(licenses).as_posix()] = regular_bytes(path)
    required = {"NAM/LICENSE", "Eigen/COPYING.MPL2", "SpeexDSP/COPYING"} | {"ssh/" + n for n in SSH_LICENSES}
    if not required <= files.keys() or any(not data for data in files.values()):
        raise RuntimeError("Build is missing required nonempty dependency licenses")
    expected = (provenance.get("ssh") or {}).get("licenses")
    if (not isinstance(expected, dict) or not SSH_LICENSES <= expected.keys()
            or any(digest_bytes(files.get("ssh/" + name, b"")) != sha for name, sha in expected.items())):
        raise RuntimeError("SSH license hashes differ from build provenance")
    return files


def plan(build_dir):
    build_dir = build_dir.resolve()
    manifest = json.loads(regular_bytes(build_dir / "assets/manifest.json"))
    release = load_release()
    if manifest.get("schema") != "tmp-player-build-v1":
        raise RuntimeError("Unsupported player build manifest")
    if (manifest.get("firmware_sha256") != release["firmware"]["sha256"]
            or manifest.get("engine_sha256") != release["engine_sha256"]):
        raise RuntimeError("Build targets a different firmware/engine")
    build = manifest.get("build") or {}
    if build.get("nam_build_pass") != "perf":
        raise RuntimeError("Release requires recorded perf build provenance")
    current = compiled_inputs()
    recorded = {k: v for k, v in (build.get("source_sha256") or {}).items() if k in current}
    if recorded != current:
        raise RuntimeError("Build is stale: compiled-input hashes differ from the checkout")
    for name, kind in (("git_revision", str), ("dependencies", str), ("compiler_manifest", dict), ("ssh", dict)):
        if not isinstance(build.get(name), kind) or not build[name]:
            raise RuntimeError(f"Build provenance is missing {name}")

    writes = {}
    for key, filename in BINARIES.items():
        art = manifest["artifacts"][key]
        if art.get("file") != filename:
            raise RuntimeError(f"Unexpected artifact file for {key}")
        data = regular_bytes(build_dir / "assets" / filename)
        if len(data) != art["bytes"] or digest_bytes(data) != art["sha256"]:
            raise RuntimeError(f"Build asset changed after it was recorded: {key}")
        if key != "nam_dispatch":
            out = build["ssh"].get("outputs", {}).get(key, {})
            if out.get("sha256") != art["sha256"] or out.get("size") != art["bytes"]:
                raise RuntimeError(f"SSH output differs from build provenance: {key}")
        release["assets"][key].update(bytes=art["bytes"], sha256=art["sha256"])
        writes[Path("bin") / filename] = (data, 0o755)
    licenses = validated_licenses(build_dir, build)
    for name, data in licenses.items():
        writes[Path("licenses") / name] = (data, 0o644)
    release.update(
        source_sha256=current,
        build=build,
        licenses_sha256={name: digest_bytes(data) for name, data in licenses.items()},
        validation=("Performance build provenance, current compiler-input hashes and artifact hashes "
                    "verified. Physical boot, NAM audio and root access have not been validated by "
                    "this publisher."),
    )
    writes[Path("release.json")] = ((json.dumps(release, indent=2) + "\n").encode(), 0o644)
    return writes, current


def publish(build_dir):
    writes, sources = plan(build_dir)
    originals = {}
    for name in writes:
        path = DEVICE / name
        if any(p.is_symlink() for p in [path, *path.parents] if p != DEVICE and DEVICE in p.parents):
            raise RuntimeError(f"Destination must not contain symlinks: {name}")
        originals[name] = (path.read_bytes(), path.stat().st_mode & 0o777) if path.exists() else None
    replaced, created = [], []
    with tempfile.TemporaryDirectory(prefix=".release-", dir=ROOT) as temporary:
        stage = Path(temporary)
        for i, (name, (data, mode)) in enumerate(writes.items()):
            (stage / str(i)).write_bytes(data)
            (stage / str(i)).chmod(mode)
        if compiled_inputs() != sources:
            raise RuntimeError("Player sources changed during release preparation")
        try:
            # release.json last, so a failure never leaves pins pointing at missing bytes.
            order = sorted(enumerate(writes), key=lambda item: item[1] == Path("release.json"))
            for i, name in order:
                dest = DEVICE / name
                missing, parent = [], dest.parent
                while not parent.exists():
                    missing.append(parent)
                    parent = parent.parent
                for p in reversed(missing):
                    p.mkdir()
                    created.append(p)
                os.replace(stage / str(i), dest)
                replaced.append(name)
        except BaseException:
            for j, name in enumerate(reversed(replaced)):
                old = originals[name]
                if old is None:
                    (DEVICE / name).unlink()
                else:
                    backup = stage / f"rollback-{j}"
                    backup.write_bytes(old[0])
                    backup.chmod(old[1])
                    os.replace(backup, DEVICE / name)
            for p in reversed(created):
                p.rmdir()
            raise


def check(build_dir):
    """Fail unless the build's binaries and licenses equal the checked-in device/ files."""
    writes, _ = plan(build_dir)
    differ = [str(name) for name, (data, _) in writes.items() if name != Path("release.json")
              and (not (DEVICE / name).is_file() or (DEVICE / name).read_bytes() != data)]
    if differ:
        raise RuntimeError("Build differs from the checked-in device/: " + ", ".join(differ))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--build", type=Path, required=True, help="output directory of build_player.py")
    parser.add_argument("--check", action="store_true",
                        help="write nothing; fail unless the build reproduces the checked-in binaries")
    args = parser.parse_args()
    if args.check:
        check(args.build.expanduser())
        print("Build reproduces the checked-in device/ binaries and licenses.")
        return
    publish(args.build.expanduser())
    print(f"Published: {RELEASE}")
    print("Rebuild the app/CLI (release.json is compiled in) and run: cargo test -p tmp-sdcard")
    print("No physical device validation was performed.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, KeyError) as exc:
        raise SystemExit(str(exc)) from exc
