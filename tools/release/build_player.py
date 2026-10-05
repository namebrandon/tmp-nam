#!/usr/bin/env python3
"""Build the card's ARM executables (NAM player + Dropbear) from source.

macOS on Apple Silicon, native developer tools and internet access are required.
Every run starts from an empty dependency/build directory; no tracked executable
or previous output is reused. The result is a build directory for
`tools/release/publish_release.py`:

    OUTPUT/assets/{nam_dispatch.so,dropbear,dropbearkey,manifest.json}
    OUTPUT/licenses/...
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import BINARIES, ROOT, compiled_inputs, digest, load_release  # noqa: E402


def run_logged(command, env, log):
    print("Building: " + log.name, flush=True)
    with log.open("w") as output:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
    if result.returncode:
        print("\n".join(log.read_text(errors="replace").splitlines()[-30:]), file=sys.stderr)
        raise RuntimeError(f"Build failed ({result.returncode}); full log: {log}")


def collect_licenses(work, licenses):
    shutil.copytree(work / "ssh/licenses", licenses / "ssh")
    for label, dep in (("NAM", work / "vendor/NeuralAmpModelerCore"),
                       ("Eigen", work / "vendor/NeuralAmpModelerCore/Dependencies/eigen"),
                       ("SpeexDSP", work / "vendor/SpeexDSP")):
        for pattern in ("LICENSE*", "COPYING*", "AUTHORS*"):
            for path in dep.glob(pattern):
                if path.is_file():
                    dest = licenses / label / path.name
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(path, dest)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, required=True, help="new build directory (never overwritten)")
    parser.add_argument("--work-dir", type=Path, help="new empty dependency/build directory")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--pass", dest="build_pass", choices=("parity", "perf"), default="perf")
    args = parser.parse_args()
    output = args.output.expanduser().resolve()
    if output.exists():
        parser.error("output directory already exists; choose a new destination")
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    output.parent.mkdir(parents=True, exist_ok=True)
    work = (args.work_dir.expanduser().resolve() if args.work_dir
            else Path(tempfile.mkdtemp(prefix="player-build-", dir=output.parent)))
    if work == output or work in output.parents or output in work.parents:
        parser.error("output and work directories must not contain each other")
    if work.exists() and any(work.iterdir()):
        parser.error("--work-dir must be empty; a source rebuild never reuses old binaries")
    work.mkdir(parents=True, exist_ok=True)
    logs = work / "logs"
    logs.mkdir()

    release = load_release()
    inputs = compiled_inputs()
    env = dict(os.environ, TMP_NAM_VENDOR_DIR=str(work / "vendor"), NAM_BUILD_PASS=args.build_pass)
    env.pop("TMP_NAM_TOOLCHAIN_DIR", None)  # never inherit a previous toolchain tree
    for name, command in [
        ("dependencies", ["bash", "player/bootstrap_nam_deps.sh"]),
        ("toolchain", ["bash", "player/bootstrap_nam_toolchain.sh"]),
        ("dispatcher", ["bash", "player/build_nam_dispatch.sh"]),
        ("ssh", ["bash", "player/build_dropbear.sh", "--output-dir", str(work / "ssh"), "--jobs", str(args.jobs)]),
    ]:
        run_logged(command, env, logs / (name + ".log"))
    if compiled_inputs() != inputs:
        raise RuntimeError("Player sources changed during the build; rebuild from a stable checkout")

    provenance = {
        "git_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "nam_build_pass": args.build_pass,
        "source_sha256": inputs,
        "dependencies": (ROOT / "player/stubs/vendor/VERSION").read_text(),
        "compiler_manifest": json.loads((ROOT / "player/toolchains/messense-v1.1.0.json").read_text()),
        "ssh": json.loads((work / "ssh/provenance.json").read_text()),
    }
    with tempfile.TemporaryDirectory(prefix="player-stage-", dir=output.parent) as temporary:
        stage = Path(temporary) / "build"
        assets = stage / "assets"
        assets.mkdir(parents=True)
        sources = {
            "nam_dispatch": work / f"vendor/build/dispatch-{args.build_pass}-a57/nam_dispatch.so",
            "dropbear": work / "ssh/dropbear",
            "dropbearkey": work / "ssh/dropbearkey",
        }
        artifacts = {}
        for key, source in sources.items():
            target = assets / BINARIES[key]
            shutil.copyfile(source, target)
            target.chmod(0o755)
            artifacts[key] = {"file": target.name, "bytes": target.stat().st_size, "sha256": digest(target)}
        manifest = {
            "schema": "tmp-player-build-v1",
            "firmware_sha256": release["firmware"]["sha256"],
            "engine_sha256": release["engine_sha256"],
            "artifacts": artifacts,
            "build": provenance,
        }
        (assets / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        collect_licenses(work, stage / "licenses")
        stage.rename(output)
    print(f"Player build: {output}\nBuild logs: {logs}\n"
          f"Publish with: python3 tools/release/publish_release.py --build {output}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as exc:
        raise SystemExit(str(exc)) from exc
