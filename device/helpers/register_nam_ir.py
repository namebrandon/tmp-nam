#!/usr/bin/env python3
"""Register every *.nam.wav in /data/userIRs/ into /data/userIRs.json so the
Tone Master Pro lists it as a selectable User IR.

Runs ON THE AMP (Python 3.5-safe: no f-strings). The firmware maps a registry
`name` to the file `<name>.wav`, so a file `X.nam.wav` is registered under the
name `X.nam` — that still resolves to the file on disk AND keeps the `.nam.wav`
suffix that nam_dispatch.so intercepts to load the neural model instead of a
plain IR. Idempotent: re-running only adds entries that aren't already present.

The firmware deletes unregistered files from /data/userIRs/ on boot, so a NAM
model must be registered here (not just copied in) to persist and appear in the
User-IR picker. Run this while tm-stomp-server is STOPPED so the engine can't
rewrite the registry out from under the edit:

    systemctl stop tm-stomp-server
    python3 /data/nam/register_nam_ir.py
    systemctl start tm-stomp-server
"""

import json
import os
import sys

IR_DIR = "/data/userIRs"
REG = "/data/userIRs.json"


def main():
    try:
        with open(REG) as f:
            data = json.load(f)
    except Exception as e:
        sys.stderr.write("cannot read %s: %s\n" % (REG, e))
        return 1
    irs = data.get("userIRs")
    if not isinstance(irs, list):
        sys.stderr.write("no userIRs array in %s\n" % REG)
        return 1

    existing = set(e.get("name") for e in irs if isinstance(e, dict))
    added = []

    if len(sys.argv) > 1:
        # Register the given .nam.wav filename(s) BY NAME, even if the file is
        # not on disk yet. The firmware's IR pruner deletes unregistered files
        # from /data/userIRs/ (even while tm-stomp-server is stopped), so the
        # name must exist in the registry BEFORE the file is pushed for the file
        # to survive. name = filename minus the trailing ".wav".
        for fn in sys.argv[1:]:
            name = fn[:-4] if fn.endswith(".wav") else fn
            if name in existing:
                continue
            irs.append({"exists": True, "name": name})
            existing.add(name)
            added.append(name)
    else:
        # No args: scan /data/userIRs/ and register any present *.nam.wav.
        try:
            names = sorted(os.listdir(IR_DIR))
        except Exception as e:
            sys.stderr.write("cannot list %s: %s\n" % (IR_DIR, e))
            return 1
        for fn in names:
            if not fn.endswith(".nam.wav"):
                continue
            name = fn[:-4]           # strip ".wav" -> "<...>.nam"
            if name in existing:
                continue
            irs.append({"exists": True, "name": name})
            existing.add(name)
            added.append(name)

    if added:
        tmp = REG + ".tmp"
        with open(tmp, "w") as f:
            json.dump(data, f, indent=4)
        os.rename(tmp, REG)      # atomic replace

    print("added %d entr%s: %s" % (len(added),
                                   "y" if len(added) == 1 else "ies", added))
    print("total userIRs now: %d" % len(irs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
