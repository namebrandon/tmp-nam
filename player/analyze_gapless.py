#!/usr/bin/env python3
"""Analyze a preset-switching capture of the TMP's USB output for gaps.

Input: <prefix>.wav (all USB-Out channels, f32) and <prefix>.json (switch frames),
as written by the local-only Companion `probe --gapless`. Channels 0/1 are the
processed stereo pair, channel 2 the dry instrument (a usable reference only in
loopback mode: re-amp does not return it, and re-amp also latches the preset at
engage, so switch tests must use loopback), 3 if present the remaining channel.

Per switch it reports, over the window from the switch to the next one:
  - zero_runs: runs of >= 1 ms where both processed channels are exactly 0.0
  - atten_runs: runs of >= 2 ms where every channel with signal sits > 20 dB
    below its own median 1 ms RMS of the surrounding second (an interface-level
    dropout, as opposed to a preset level change)
  - min_wet_db: lowest processed 1 ms RMS relative to the dry 1 ms RMS, after the
    first 50 ms (the firmware crossfade)
  - wet_db / centroid_hz: the dwell's processed level and spectral centroid, which
    must differ between unlike presets (proves the switch was audible)
Exit status 1 if any zero or attenuation run is found.
"""
import argparse
import json
import struct
import sys

import numpy as np


def read_wav(path):
    with open(path, "rb") as f:
        data = f.read()
    i, fmt, channels, rate = 12, None, 0, 0
    while i < len(data):
        cid = data[i:i + 4]
        n = struct.unpack("<I", data[i + 4:i + 8])[0]
        body = data[i + 8:i + 8 + n]
        if cid == b"fmt ":
            fmt, channels, rate = struct.unpack("<HHI", body[:8])
        elif cid == b"data":
            if fmt not in (3, 65534):
                raise SystemExit("expected float WAV")
            x = np.frombuffer(body, "<f4")
            return x.reshape(-1, channels), rate
        i += 8 + n + (n & 1)
    raise SystemExit("no data chunk")


def rms_1ms(x, rate):
    hop = rate // 1000
    n = len(x) // hop
    return np.sqrt(np.mean(np.square(x[: n * hop].reshape(n, hop, -1), dtype=np.float64), axis=1))  # (ms, ch)


def runs(mask, min_len):
    """Number of runs of True at least min_len long."""
    edges = np.diff(np.r_[0, mask.astype(np.int8), 0])
    return int(np.count_nonzero(np.flatnonzero(edges == -1) - np.flatnonzero(edges == 1) >= min_len))


def db(v):
    return 20 * np.log10(np.maximum(v, 1e-12))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("prefix")
    ap.add_argument("--json-out")
    args = ap.parse_args()
    audio, rate = read_wav(args.prefix + ".wav")
    log = json.load(open(args.prefix + ".json"))
    sw = log["switches"]
    env = rms_1ms(audio, rate)
    wet = np.sqrt((env[:, 0] ** 2 + env[:, 1] ** 2) / 2)
    dry = env[:, 2]
    hop = rate // 1000
    rows = []
    for k, e in enumerate(sw):
        # The capture begins before the stream carries audio; skip its first 200 ms.
        start = max(e["frame"] // hop, 200 if k == 0 else 0)
        end = (sw[k + 1]["frame"] // hop) if k + 1 < len(sw) else len(wet)
        # The settled level and spectrum start 200 ms into the dwell; keep 100 ms of it.
        if end - start < 300:
            continue
        seg = audio[start * hop:end * hop]
        zero = (seg[:, 0] == 0.0) & (seg[:, 1] == 0.0)
        zero_runs = runs(zero, hop)
        ctx = env[max(0, start - 500):min(len(env), end + 500)]
        med = np.median(ctx, axis=0)
        live = med > 1e-6
        w = env[start:end][:, live]
        atten = np.all(w < med[live] * 0.1, axis=1) if live.any() else np.zeros(end - start, bool)
        atten_runs = runs(atten, 2)
        settled = slice(start + 50, end)
        ratio = db(wet[settled]) - db(dry[settled])
        dwell = audio[(start + 200) * hop:end * hop, :2].mean(axis=1, dtype=np.float64)
        centroid = 0.0
        if len(dwell) > 256:
            spec = np.abs(np.fft.rfft(dwell * np.hanning(len(dwell))))
            centroid = float((spec * np.fft.rfftfreq(len(dwell), 1 / rate)).sum() / max(spec.sum(), 1e-12))
        row = {"slot": e["slot"], "from": sw[k - 1]["slot"] if k else None, "zero_runs": zero_runs,
               "atten_runs": atten_runs, "min_wet_db": float(ratio.min()),
               "wet_db": float(db(np.sqrt(np.mean(wet[start + 200:end] ** 2)))), "centroid_hz": centroid}
        rows.append(row)
        print("%-9s -> %-3s zero_runs=%d atten_runs=%d min_wet_rel=%6.1f dB  level=%6.1f dB  centroid=%5.0f Hz" % (
            "start" if row["from"] is None else row["from"], row["slot"], zero_runs, atten_runs,
            row["min_wet_db"], row["wet_db"], centroid))
    if not rows:
        raise SystemExit("no dwell of at least 300 ms to analyze")
    summary = {"switches": len(sw) - 1, "dwells": len(rows), "zero_runs": sum(r["zero_runs"] for r in rows),
               "atten_runs": sum(r["atten_runs"] for r in rows),
               "min_wet_db": min(r["min_wet_db"] for r in rows), "rows": rows}
    print("TOTAL switches=%d dwells=%d zero_runs=%d atten_runs=%d min_wet_rel=%.1f dB" % (
        summary["switches"], summary["dwells"], summary["zero_runs"], summary["atten_runs"], summary["min_wet_db"]))
    if args.json_out:
        json.dump(summary, open(args.json_out, "w"), indent=2)
    return 1 if summary["zero_runs"] + summary["atten_runs"] else 0


if __name__ == "__main__":
    sys.exit(main())
