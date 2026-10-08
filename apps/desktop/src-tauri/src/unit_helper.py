# Device-side helper for the TMP NAM desktop app.
# Pushed to /tmp on every console connection and run ON THE AMP.
# Python 3.5-safe: no f-strings, no pathlib. Keep every line short:
# the console reads lines through BusyBox ash.
import base64
import hashlib
import json
import os
import sys
import tempfile
import time

IR_DIR = "/data/userIRs"
REG = "/data/userIRs.json"
PLAYER = "/data/nam/player.json"
INDEX = "/data/nam/.app-index.json"
SUFFIX = ".nam.wav"


def u(name):
    # os.listdir names are surrogate-escaped under the unit's ASCII locale;
    # decode the raw bytes as UTF-8 so the JSON reply stays valid.
    try:
        return name.encode("ascii", "surrogateescape").decode("utf-8", "replace")
    except Exception:
        return name


def emit(obj):
    sys.stdout.write(json.dumps(obj, sort_keys=True) + "\n")


def read_json(path, default):
    try:
        with open(path) as f:
            return json.load(f)
    except Exception:
        return default


def write_json(path, data, indent=None):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f, indent=indent)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    os.rename(tmp, path)


def sha256_path(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(1 << 20)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


# Bump when describe() reports something new (2: A1 network width).
DESCRIBE_VERSION = 2


def child_summary(model):
    cfg = model.get("config") or {}
    out = {"architecture": model.get("architecture")}
    layers = cfg.get("layers")
    if isinstance(layers, list) and layers:
        first = layers[0]
        if isinstance(first, dict):
            out["channels"] = first.get("channels")
    return out


def describe(path):
    with open(path) as f:
        data = json.load(f)
    arch = data.get("architecture")
    meta = data.get("metadata") or {}
    info = {
        "architecture": arch,
        "version": data.get("version"),
        "sample_rate": data.get("sample_rate"),
        "meta": {
            "name": meta.get("name"),
            "modeled_by": meta.get("modeled_by"),
            "gear_make": meta.get("gear_make"),
            "gear_model": meta.get("gear_model"),
            "gear_type": meta.get("gear_type"),
            "tone_type": meta.get("tone_type"),
        },
        "submodels": [],
    }
    cfg = data.get("config") or {}
    subs = cfg.get("submodels") if isinstance(cfg, dict) else None
    if arch == "SlimmableContainer" and isinstance(subs, list):
        for item in subs:
            if not isinstance(item, dict):
                continue
            child = item.get("model") or {}
            row = child_summary(child)
            row["max_value"] = item.get("max_value", 1.0)
            info["submodels"].append(row)
    else:
        # A1: the network width names its size (16 Standard, 12 Lite, 8 Feather, 4 Nano).
        info["channels"] = child_summary(data).get("channels")
    return info


def cmd_info(_args):
    build = None
    try:
        with open("/etc/sd-root-build-id") as f:
            raw = f.read().strip()
        try:
            build = json.loads(raw).get("build_id") or raw
        except ValueError:
            build = raw
    except Exception:
        pass
    dispatch = "/usr/local/lib/nam_dispatch.so"
    emit({
        "build_id": build,
        "dispatch_sha256": (sha256_path(dispatch)
                            if os.path.isfile(dispatch) else None),
        "python": sys.version.split()[0],
    })


def registered_names():
    reg = read_json(REG, {})
    irs = reg.get("userIRs") if isinstance(reg, dict) else None
    if not isinstance(irs, list):
        return set()
    return set(e.get("name") for e in irs
               if isinstance(e, dict) and e.get("name"))


def listed_options(entry):
    # Only expose the numeric options the desktop understands. Invalid values
    # must not make the entire list undecodable by the Rust backend.
    if not isinstance(entry, dict):
        raise ValueError("expected a capture settings object")
    options = {}
    for key, maximum in (("size", 1), ("output_gain", 8)):
        if key in entry:
            value = entry[key]
            if (isinstance(value, bool) or not isinstance(value, (int, float))
                    or not 0 <= value <= maximum):
                raise ValueError("invalid " + key)
            options[key] = value
    return options


def cmd_list(_args):
    names = registered_names()
    settings_error = None
    try:
        with open(PLAYER, "rb") as f:
            player = json.loads(f.read().decode("utf-8"))
        if not isinstance(player, dict) or not isinstance(player.get("models"), dict):
            raise ValueError("expected a models object")
        opts = player["models"]
        if any(not isinstance(entry, dict) for entry in opts.values()):
            settings_error = "Invalid capture settings in %s." % PLAYER
    except FileNotFoundError:
        opts = {}
        if os.path.lexists(PLAYER):
            settings_error = "Cannot read %s." % PLAYER
    except OSError:
        opts = {}
        settings_error = "Cannot read %s." % PLAYER
    except ValueError:
        opts = {}
        settings_error = "Invalid player settings in %s." % PLAYER
    index = read_json(INDEX, {})
    if not isinstance(index, dict):
        index = {}
    fresh = {}
    rows = []
    try:
        files = sorted(os.listdir(IR_DIR))
    except Exception:
        files = []
    for fn in files:
        if not fn.endswith(SUFFIX):
            continue
        path = os.path.join(IR_DIR, fn)
        st = os.stat(path)
        # DESCRIBE_VERSION in the key: a describe() change refreshes the cache.
        key = "%s|%d|%d|%d" % (fn, st.st_size, int(st.st_mtime), DESCRIBE_VERSION)
        cached = index.get(key)
        if not isinstance(cached, dict):
            cached = {"sha256": sha256_path(path)}
            try:
                cached["info"] = describe(path)
            except Exception as e:
                cached["error"] = str(e)[:200]
        fresh[key] = cached
        name = fn[:-4]
        row = {
            "name": u(name),
            "file": u(fn),
            "bytes": st.st_size,
            "registered": u(name) in names,
            "present": True,
        }
        row.update(cached)
        sel = opts.get(cached.get("sha256"), {})
        try:
            row["options"] = listed_options(sel)
        except ValueError:
            row["options"] = {}
            settings_error = "Invalid capture settings in %s." % PLAYER
        rows.append(row)
    for name in sorted(names):
        if name.endswith(".nam") and not os.path.isfile(
                os.path.join(IR_DIR, name + ".wav")):
            rows.append({"name": name, "file": name + ".wav",
                         "bytes": 0, "registered": True,
                         "present": False, "options": {}})
    if fresh != index:
        try:
            write_json(INDEX, fresh)
        except Exception:
            pass
    result = {"models": rows}
    if settings_error:
        result["settings_error"] = settings_error
    emit(result)


def cmd_register(args):
    reg = read_json(REG, None)
    if not isinstance(reg, dict) or not isinstance(reg.get("userIRs"), list):
        raise SystemExit("cannot read %s" % REG)
    irs = reg["userIRs"]
    existing = set(e.get("name") for e in irs if isinstance(e, dict))
    added = []
    for name in args:
        if name not in existing:
            irs.append({"exists": True, "name": name})
            existing.add(name)
            added.append(name)
    if added:
        write_json(REG, reg, indent=4)
    emit({"added": added})


def cmd_unregister(args):
    reg = read_json(REG, None)
    if not isinstance(reg, dict) or not isinstance(reg.get("userIRs"), list):
        raise SystemExit("cannot read %s" % REG)
    drop = set(args)
    before = len(reg["userIRs"])
    reg["userIRs"] = [e for e in reg["userIRs"]
                      if not (isinstance(e, dict) and e.get("name") in drop)]
    if len(reg["userIRs"]) != before:
        write_json(REG, reg, indent=4)
    removed = []
    for name in args:
        path = os.path.join(IR_DIR, name + ".wav")
        if os.path.isfile(path):
            os.remove(path)
            removed.append(name)
    emit({"removed": removed})


def cmd_persisted(args):
    # persisted <timeout s> (+name | -name)...: wait until the engine's
    # saved registry lists every +name and no -name.
    deadline = time.time() + float(args[0])
    want = [(a[0] == "+", a[1:]) for a in args[1:]]
    while True:
        names = registered_names()
        if all((n in names) == present for present, n in want):
            emit({"persisted": True})
            return
        if time.time() > deadline:
            raise SystemExit("the unit did not save its IR list")
        time.sleep(0.1)


def cmd_install(args):
    # install <b64 file> <registry name> <sha256>
    src, name, want = args[0], args[1], args[2]
    if not name.endswith(".nam") or "/" in name:
        raise SystemExit("bad name %s" % name)
    with open(src, "rb") as f:
        raw = base64.b64decode(f.read())
    got = hashlib.sha256(raw).hexdigest()
    if got != want:
        raise SystemExit("hash mismatch %s != %s" % (got, want))
    dest = os.path.join(IR_DIR, name + ".wav")
    tmp = dest + ".tmp.%d" % os.getpid()
    with open(tmp, "wb") as f:
        f.write(raw)
        f.flush()
        os.fsync(f.fileno())
    os.rename(tmp, dest)
    os.remove(src)
    emit({"installed": name, "bytes": len(raw)})


def backup_player(raw):
    # Exclusive creation keeps earlier backups intact. Copy before replacing:
    # a failed backup or atomic save must leave the original file in place.
    fd, path = tempfile.mkstemp(prefix="player.json.invalid.",
                                dir=os.path.dirname(PLAYER))
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(raw)
            f.flush()
            os.fsync(f.fileno())
    except OSError:
        try:
            os.remove(path)
        except OSError:
            pass
        raise
    return path


def cmd_opts(args):
    # opts <sha256> <size|-|=> <gain|-|=>; = keeps, - removes.
    sha, size, gain = args[0], args[1], args[2]
    values = [(key, None if value == "-" else float(value))
              for key, value in (("size", size), ("output_gain", gain))
              if value != "="]
    if not values:
        emit({"unchanged": True})
        return
    raw = None
    try:
        with open(PLAYER, "rb") as f:
            raw = f.read()
    except FileNotFoundError:
        if os.path.lexists(PLAYER):
            raise SystemExit("cannot read %s; settings were not changed" % PLAYER)
    except OSError:
        raise SystemExit("cannot read %s; settings were not changed" % PLAYER)
    recovery = None
    if raw is None:
        data = {"models": {}}
    else:
        try:
            data = json.loads(raw.decode("utf-8"))
        except ValueError:
            data = None
        if not isinstance(data, dict) or not isinstance(data.get("models"), dict):
            data = {"models": {}}
            recovery = "Invalid player settings were reset; other captures use defaults."
    models = data["models"]
    entry = models.get(sha, {})
    if not isinstance(entry, dict):
        entry = {}
        recovery = "Invalid settings for this capture were reset."
    for key, value in values:
        if value is None:
            entry.pop(key, None)
        else:
            entry[key] = value
    if entry:
        models[sha] = entry
    else:
        models.pop(sha, None)
    backup = None
    if recovery:
        try:
            backup = backup_player(raw)
        except OSError:
            raise SystemExit("cannot back up %s; settings were not changed" % PLAYER)
    try:
        write_json(PLAYER, data, indent=2)
    except OSError:
        if backup:
            raise SystemExit("cannot save settings; original retained; backup: %s" % backup)
        raise
    result = {"options": entry}
    if backup:
        result["warning"] = "%s Backup: %s. Reselect the capture on the unit." % (recovery, backup)
    emit(result)


COMMANDS = {
    "info": cmd_info,
    "list": cmd_list,
    "register": cmd_register,
    "unregister": cmd_unregister,
    "persisted": cmd_persisted,
    "install": cmd_install,
    "opts": cmd_opts,
}

if __name__ == "__main__":
    try:
        COMMANDS[sys.argv[1]](sys.argv[2:])
    except SystemExit as e:
        emit({"error": str(e)})
        sys.exit(1)
    except Exception as e:
        emit({"error": "%s: %s" % (type(e).__name__, e)})
        sys.exit(1)
