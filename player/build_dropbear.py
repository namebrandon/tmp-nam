#!/usr/bin/env python3
"""Build static AArch64 Dropbear tools from verified archives in a fresh directory."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shlex
import shutil
import struct
import subprocess
import tarfile
import tempfile
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from urllib.parse import urlparse
from urllib.request import Request, urlopen

PLAYER_DIR = Path(__file__).resolve().parent
MANIFEST = PLAYER_DIR / "toolchains/dropbear-source.json"


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def host_key() -> str:
    machine = {"arm64": "aarch64", "amd64": "x86_64"}.get(platform.machine(), platform.machine())
    system = {"Darwin": "macos", "Linux": "linux"}.get(platform.system(), platform.system())
    return f"{machine}-{system}"


def fetch_archive(spec: dict, cache: Path, offline: bool) -> Path:
    url = spec["url"]
    parsed = urlparse(url)
    if parsed.scheme != "https":
        raise ValueError("Source archives must use HTTPS")
    archive = cache / Path(parsed.path).name
    if not archive.is_file():
        if offline:
            raise FileNotFoundError(f"Offline archive missing: {archive}")
        cache.mkdir(parents=True, exist_ok=True)
        partial = archive.with_name(archive.name + ".partial")
        print(f"Downloading {archive.name}", flush=True)
        try:
            request = Request(url, headers={"User-Agent": "TMP-source-build/1"})
            with urlopen(request, timeout=60) as response, partial.open("wb") as output:
                shutil.copyfileobj(response, output)
            if sha256(partial) != spec["sha256"]:
                raise ValueError(f"Archive SHA-256 mismatch: {url}")
            partial.replace(archive)
        finally:
            partial.unlink(missing_ok=True)
    if sha256(archive) != spec["sha256"]:
        raise ValueError(f"Cached archive SHA-256 mismatch: {archive}")
    return archive


def extract_archive(archive: Path, destination: Path) -> None:
    with tarfile.open(archive) as source:
        for member in source.getmembers():
            name = PurePosixPath(member.name)
            if name.is_absolute() or ".." in name.parts or member.isdev():
                raise ValueError(f"Unsafe archive member: {member.name}")
            if member.issym() or member.islnk():
                link = PurePosixPath(member.linkname)
                relative = name.parent / link if member.issym() else link
                target = (destination / relative).resolve()
                if link.is_absolute() or not target.is_relative_to(destination.resolve()):
                    raise ValueError(f"Unsafe archive link: {member.name}")
        # Python 3.11 installations without the backported data filter still get
        # the member/link bounds checks above, on an already hash-verified archive.
        if hasattr(tarfile, "data_filter"):
            source.extractall(destination, filter="data")
        else:
            source.extractall(destination)


def validate_static_aarch64(path: Path) -> dict:
    data = path.read_bytes()
    if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01":
        raise ValueError(f"Expected ELF64 little-endian binary: {path}")
    elf_type, machine = struct.unpack_from("<HH", data, 16)
    if elf_type not in (2, 3) or machine != 183:
        raise ValueError(f"Expected AArch64 executable: {path}")
    entry, phoff = struct.unpack_from("<QQ", data, 24)
    phentsize, phnum = struct.unpack_from("<HH", data, 54)
    if phentsize != 56 or not phnum or phoff + phentsize * phnum > len(data):
        raise ValueError(f"Invalid ELF program headers: {path}")
    executable_entry = False
    for index in range(phnum):
        kind, flags, offset, vaddr, _, filesz, memsz, _ = struct.unpack_from("<IIQQQQQQ", data, phoff + index * phentsize)
        if offset + filesz > len(data):
            raise ValueError(f"ELF segment extends beyond file: {path}")
        if kind == 3:
            raise ValueError(f"Static binary must not have PT_INTERP: {path}")
        if kind == 2:
            if filesz % 16:
                raise ValueError(f"Invalid ELF dynamic entries: {path}")
            for position in range(offset, offset + filesz, 16):
                tag, _ = struct.unpack_from("<qQ", data, position)
                if tag == 1:
                    raise ValueError(f"Static binary must not have DT_NEEDED: {path}")
                if tag == 0:
                    break
        if kind == 1 and flags & 1 and vaddr <= entry < vaddr + memsz:
            executable_entry = True
    if not executable_entry:
        raise ValueError(f"ELF entry point is outside executable load segments: {path}")
    return {"class": "ELF64", "byte_order": "little", "machine": "AArch64", "type": "EXEC" if elf_type == 2 else "DYN",
            "linkage": "static" if elf_type == 2 else "static-pie",
            "interpreter": None, "needed": [], "sha256": sha256(path), "size": len(data)}


def run(command: list[str], cwd: Path, env: dict, log: Path) -> None:
    with log.open("w") as stream:
        stream.write("Command: " + shlex.join(command) + "\n")
        stream.flush()
        result = subprocess.run(command, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"Command failed ({result.returncode}); see {log}")


def build(args: argparse.Namespace) -> Path:
    manifest = json.loads(MANIFEST.read_text())
    host = host_key()
    if host not in manifest["zig_archives"]:
        raise ValueError(f"No pinned Zig compiler for {host}")
    if not shutil.which("make"):
        raise ValueError("Missing host make")
    # Dropbear's INSTALL.md: "Binaries can be stripped with `make strip`" (advised for
    # STATIC=1). Zig ships no strip, so use the pinned cross toolchain's GNU strip
    # (bootstrap_nam_toolchain.sh). Stripping also drops debug info, which would
    # otherwise embed the absolute build path.
    strip = args.toolchain_dir / "bin/aarch64-unknown-linux-gnu-strip"
    if not os.access(strip, os.X_OK):
        raise ValueError(f"Missing {strip}; run player/bootstrap_nam_toolchain.sh first")
    compiler_spec = manifest["zig_archives"][host]
    source_archive = fetch_archive(manifest["source"], args.cache_dir, args.offline)
    compiler_archive = fetch_archive(compiler_spec, args.cache_dir, args.offline)
    args.build_dir.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="run-", dir=args.build_dir))
    print(f"Fresh build: {run_dir}", flush=True)
    extract_archive(source_archive, run_dir)
    extract_archive(compiler_archive, run_dir)
    source = run_dir / manifest["source"]["directory"]
    compiler = run_dir / compiler_spec["directory"]
    zig = compiler / "zig"
    version = subprocess.check_output([str(zig), "version"], text=True).strip()
    if version != manifest["zig_version"]:
        raise ValueError(f"Unexpected Zig version: {version}")
    wrappers = run_dir / "tools"
    wrappers.mkdir()
    commands = {"CC": [str(zig), "cc", "-target", manifest["target"]],
                "AR": [str(zig), "ar"], "RANLIB": [str(zig), "ranlib"]}
    env = os.environ.copy()
    # Prevent unrelated host build flags from altering the recorded target build.
    for name in ("CC", "CXX", "AR", "RANLIB", "CFLAGS", "CPPFLAGS", "LDFLAGS", "LIBS", "CONFIG_SITE"):
        env.pop(name, None)
    for name, command in commands.items():
        wrapper = wrappers / name.lower()
        wrapper.write_text("#!/bin/sh\nexec " + shlex.join(command) + ' "$@"\n')
        wrapper.chmod(0o755)
        env[name] = shlex.quote(str(wrapper))
    env.update({"ZIG_GLOBAL_CACHE_DIR": str(run_dir / "zig-cache"),
                "ZIG_LOCAL_CACHE_DIR": str(run_dir / "zig-local-cache"),
                "CFLAGS": "-Os", "LC_ALL": "C", "SOURCE_DATE_EPOCH": "1729608480"})
    configure = [str(source / "configure"), *manifest["configure_args"]]
    make = ["make", f"-j{args.jobs}", *manifest["make_args"]]
    print("Configuring Dropbear 2024.86 for static AArch64/musl", flush=True)
    run(configure, source, env, run_dir / "configure.log")
    print("Compiling dropbear and dropbearkey", flush=True)
    run(make, source, env, run_dir / "build.log")
    print("Stripping dropbear and dropbearkey", flush=True)
    run(["make", "strip", f"STRIP={strip}", *manifest["make_args"]], source, env, run_dir / "strip.log")
    outputs = {name: validate_static_aarch64(source / name) for name in ("dropbear", "dropbearkey")}
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name in outputs:
        shutil.copy2(source / name, args.output_dir / name)
    licenses = args.output_dir / "licenses"
    licenses.mkdir(exist_ok=True)
    license_inputs = {"dropbear-LICENSE": source / "LICENSE", "libtomcrypt-LICENSE": source / "libtomcrypt/LICENSE",
                      "libtommath-LICENSE": source / "libtommath/LICENSE", "musl-COPYRIGHT": compiler / "lib/libc/musl/COPYRIGHT",
                      "zig-LICENSE": compiler / "LICENSE"}
    for name, path in license_inputs.items():
        shutil.copyfile(path, licenses / name)
    for name in ("configure.log", "build.log", "strip.log"):
        shutil.copyfile(run_dir / name, args.output_dir / name)
    shutil.copyfile(source / "config.log", args.output_dir / "config.log")
    provenance = {"schema": 1, "created_utc": datetime.now(timezone.utc).isoformat(),
                  "source": manifest["source"], "compiler": {**compiler_spec, "version": version},
                  "target": manifest["target"], "host": host, "configure_args": manifest["configure_args"],
                  "make_args": manifest["make_args"], "cflags": env["CFLAGS"],
                  "recipe_sha256": sha256(Path(__file__)), "manifest_sha256": sha256(MANIFEST),
                  "strip": {"tool": strip.name, "sha256": sha256(strip)},
                  "build_directory": run_dir.name, "outputs": outputs,
                  "licenses": {name: sha256(path) for name, path in license_inputs.items()}}
    (args.output_dir / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    print(f"Static ELF validation passed; outputs: {args.output_dir}", flush=True)
    return args.output_dir


def main() -> None:
    vendor = Path(os.environ.get("TMP_NAM_VENDOR_DIR", PLAYER_DIR / "stubs/vendor"))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=vendor / "build/sd-downloads")
    parser.add_argument("--build-dir", type=Path, default=vendor / "build/dropbear-2024.86-builds")
    parser.add_argument("--output-dir", type=Path, default=vendor / "build/dropbear-2024.86-aarch64-musl")
    parser.add_argument("--toolchain-dir", type=Path,
                        default=Path(os.environ.get("TMP_NAM_TOOLCHAIN_DIR",
                                                    vendor / "build/toolchains/messense-v1.1.0/aarch64-unknown-linux-gnu")))
    parser.add_argument("--offline", action="store_true", help="Require verified archives already in cache")
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    for name in ("cache_dir", "build_dir", "output_dir", "toolchain_dir"):
        setattr(args, name, getattr(args, name).resolve())
    try:
        build(args)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError, tarfile.TarError) as exc:
        parser.exit(1, f"Dropbear build failed: {exc}\n")


if __name__ == "__main__":
    main()
