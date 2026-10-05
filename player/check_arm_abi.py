#!/usr/bin/env python3
"""Validate the Linux/AArch64 artifact contract without executing the artifact."""
import argparse
import re
import subprocess
from pathlib import Path


def validate(header, dynamic, versions, kind, allowed, programs=""):
    errors = []
    for label, pattern in (("ELF64", r"Class:\s+ELF64"),
                           ("little endian", r"Data:.*little endian"),
                           ("AArch64", r"Machine:\s+AArch64")):
        if not re.search(pattern, header):
            errors.append("Expected " + label)
    if any(value != "ELF64" for value in re.findall(r"Class:\s+(\w+)", header)):
        errors.append("Mixed or unsupported ELF classes")
    if any("little endian" not in value for value in re.findall(r"Data:\s+([^\n]+)", header)):
        errors.append("Mixed or unsupported byte orders")
    types = re.findall(r"Type:\s+(\w+)", header)
    expected = {"REL"} if kind == "archive" else ({"DYN"} if kind == "shared" else {"DYN", "EXEC"})
    if not types or any(t not in expected for t in types):
        errors.append("Unexpected ELF type for " + kind)
    if kind == "archive":
        machines = re.findall(r"Machine:\s+([^\n]+)", header)
        if not machines or any(m.strip() != "AArch64" for m in machines):
            errors.append("Archive contains non-AArch64 members")
        return errors
    has_interpreter = bool(re.search(r"^\s*INTERP\s", programs, re.MULTILINE))
    if kind == "executable":
        if not has_interpreter:
            errors.append("Executable requires a Linux program interpreter")
        elif re.findall(r"\[Requesting program interpreter: ([^]]+)\]", programs) != ["/lib/ld-linux-aarch64.so.1"]:
            errors.append("Executable requires /lib/ld-linux-aarch64.so.1")
    if kind == "shared" and has_interpreter:
        errors.append("Shared library must not have a program interpreter")
    if re.search(r"\((?:RPATH|RUNPATH)\)", dynamic):
        errors.append("Runtime library paths are unsupported")
    needed = set(re.findall(r"Shared library: \[([^]]+)\]", dynamic))
    if not needed:
        errors.append("Expected dynamic libc dependency")
    for name in sorted(needed - set(allowed)):
        errors.append("Unsupported dependency: " + name)
    glibc = [tuple(int(n) for n in v.split(".")) for v in re.findall(r"Name: GLIBC_([0-9.]+)", versions)]
    if not glibc or any(v + (0,) * (3 - len(v)) > (2, 28, 0) for v in glibc):
        errors.append("GLIBC symbol ceiling exceeded or missing")
    if re.search(r"Name: GLIBC_[A-Za-z_]", versions):
        errors.append("Unsupported non-public GLIBC requirement")
    if re.search(r"Name: (?:GLIBCXX|CXXABI)_", versions):
        errors.append("C++ runtime must be linked statically")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--readelf", required=True)
    parser.add_argument("--kind", choices=("shared", "executable", "archive"), required=True)
    parser.add_argument("--allow", action="append", default=[])
    parser.add_argument("--reports-dir", type=Path)
    parser.add_argument("artifact", type=Path)
    args = parser.parse_args()
    outputs = {}
    for flag, suffix in (("-h", "elf"), ("-d", "dynamic"), ("-V", "versions"), ("-l", "programs")):
        if args.kind == "archive" and flag != "-h":
            outputs[suffix] = ""
            continue
        result = subprocess.run([args.readelf, flag, str(args.artifact)], text=True, capture_output=True, check=True)
        outputs[suffix] = result.stdout
        report_dir = args.reports_dir or args.artifact.parent
        report_dir.mkdir(parents=True, exist_ok=True)
        (report_dir / (args.artifact.name + "." + suffix)).write_text(result.stdout)
    errors = validate(outputs["elf"], outputs["dynamic"], outputs["versions"], args.kind, args.allow, outputs["programs"])
    if errors:
        parser.exit(1, "\n".join(errors) + "\n")
    print("ABI verified:", args.artifact)


if __name__ == "__main__":
    main()
