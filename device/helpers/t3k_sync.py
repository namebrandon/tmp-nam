#!/usr/bin/env python3
"""Pull TONE3000 NAM models onto this pedal (Python 3.5-safe).

LAN-relay OAuth: this process listens on Wi-Fi, you open the printed URL on a
phone on the same network, then A2 or A1 Feather/Nano models from your created
and favorited (bookmarked) tones are registered as User IRs.

  python3 /data/nam/t3k_sync.py           # restarts tm-stomp-server so the picker reloads
  python3 /data/nam/t3k_sync.py --no-restart

Key: /data/nam/t3k_pub
Tokens: /data/nam/t3k_tokens.json
"""
from __future__ import print_function

import base64
import hashlib
import json
import os
import socket
import sys
import threading
import time

try:
    from http.server import BaseHTTPRequestHandler, HTTPServer
    from urllib.parse import parse_qs, urlencode, urlparse
    from urllib.request import Request, urlopen
    from urllib.error import HTTPError, URLError
except ImportError:
    sys.stderr.write("python 3 required\n")
    sys.exit(1)

API = "https://www.tone3000.com/api/v1"
KEY_PATH = "/data/nam/t3k_pub"
TOKEN_PATH = "/data/nam/t3k_tokens.json"
IR_DIR = "/data/userIRs"
REG = "/data/userIRs.json"
LISTEN_PORT = 8766
ALLOWED_SIZES = ("feather", "nano")
ALLOWED_ARCH = ("1", "2", "1.0", "2.0")


def load_key():
    with open(KEY_PATH) as f:
        line = f.read().strip().splitlines()[0].strip()
    if not line.startswith("t3k_pub_"):
        raise SystemExit("bad key file %s" % KEY_PATH)
    return line


def lan_ip():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("8.8.8.8", 80))
        ip = s.getsockname()[0]
    finally:
        s.close()
    if ip.startswith("127."):
        raise SystemExit("no LAN IP; is Wi-Fi up?")
    return ip


def pkce():
    raw = os.urandom(48)
    verifier = base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")
    digest = hashlib.sha256(verifier.encode("ascii")).digest()
    challenge = base64.urlsafe_b64encode(digest).decode("ascii").rstrip("=")
    return verifier, challenge


class _CB(BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urlparse(self.path)
        if parsed.path not in ("/cb", "/callback"):
            self.send_error(404)
            return
        qs = parse_qs(parsed.query)
        self.server.result = {
            "code": (qs.get("code") or [None])[0],
            "state": (qs.get("state") or [None])[0],
            "error": (qs.get("error") or [None])[0],
            "tone_id": (qs.get("tone_id") or [None])[0],
        }
        body = (
            b"<!doctype html><meta name=viewport content='width=device-width'>"
            b"<body style='font-family:sans-serif;text-align:center;padding:2em'>"
            b"<h2>TONE3000 linked.</h2><p>Return to the pedal / Mac.</p>"
        )
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        return


def http_json(method, url, headers=None, data=None):
    hdrs = headers or {}
    body = None
    if data is not None:
        if isinstance(data, dict):
            body = urlencode(data).encode("utf-8")
            hdrs = dict(hdrs)
            hdrs.setdefault("Content-Type", "application/x-www-form-urlencoded")
        else:
            body = data
    req = Request(url, data=body, headers=hdrs)
    req.get_method = lambda: method
    try:
        resp = urlopen(req, timeout=60)
        raw = resp.read()
        ctype = resp.headers.get("Content-Type") or ""
        if "json" in ctype or (raw[:1] in (b"{", b"[")):
            return json.loads(raw.decode("utf-8"))
        return raw
    except HTTPError as e:
        err = e.read()
        raise SystemExit("HTTP %s %s\n%s" % (e.code, url, err[:500]))


def save_tokens(tok):
    tmp = TOKEN_PATH + ".tmp"
    with open(tmp, "w") as f:
        json.dump(tok, f)
    os.rename(tmp, TOKEN_PATH)
    os.chmod(TOKEN_PATH, 0o600)


def load_tokens():
    try:
        with open(TOKEN_PATH) as f:
            return json.load(f)
    except Exception:
        return None


def refresh(client_id, tok):
    rt = tok.get("refresh_token")
    if not rt:
        return None
    try:
        out = http_json(
            "POST",
            API + "/oauth/token",
            data={
                "grant_type": "refresh_token",
                "refresh_token": rt,
                "client_id": client_id,
            },
        )
    except SystemExit:
        return None
    if not isinstance(out, dict) or "access_token" not in out:
        return None
    tok.update(out)
    save_tokens(tok)
    return tok


def oauth_lan(client_id):
    ip = lan_ip()
    redirect = "http://%s:%d/cb" % (ip, LISTEN_PORT)
    verifier, challenge = pkce()
    state = base64.urlsafe_b64encode(os.urandom(16)).decode("ascii").rstrip("=")
    q = urlencode({
        "client_id": client_id,
        "redirect_uri": redirect,
        "response_type": "code",
        "code_challenge": challenge,
        "code_challenge_method": "S256",
        "state": state,
        "format": "nam",
    })
    url = API + "/oauth/authorize?" + q
    print("On your phone (same Wi-Fi as the pedal), open:\n")
    print(url)
    print("\nWaiting for TONE3000 callback on %s ..." % redirect)
    HTTPServer.allow_reuse_address = True
    srv = HTTPServer(("0.0.0.0", LISTEN_PORT), _CB)
    srv.result = None
    t = threading.Thread(target=srv.handle_request)
    t.daemon = True
    t.start()
    deadline = time.time() + 300
    while srv.result is None and time.time() < deadline:
        time.sleep(0.2)
    if srv.result is None:
        raise SystemExit("timed out waiting for phone login")
    rec = srv.result
    if rec.get("error"):
        raise SystemExit("oauth error: %s" % rec["error"])
    if rec.get("state") != state:
        raise SystemExit("state mismatch")
    if not rec.get("code"):
        raise SystemExit("no code in callback")
    tok = http_json(
        "POST",
        API + "/oauth/token",
        data={
            "grant_type": "authorization_code",
            "code": rec["code"],
            "code_verifier": verifier,
            "redirect_uri": redirect,
            "client_id": client_id,
        },
    )
    if not isinstance(tok, dict) or "access_token" not in tok:
        raise SystemExit("token exchange failed: %r" % (tok,))
    save_tokens(tok)
    return tok, rec.get("tone_id")


def api_get(tok, path):
    return http_json(
        "GET",
        API + path,
        headers={"Authorization": "Bearer " + tok["access_token"]},
    )


def safe_name(title, model_name, size):
    base = (model_name or title or "t3k").strip()
    out = []
    for ch in base:
        if ch.isalnum() or ch in "-_.":
            out.append(ch)
        else:
            out.append("-")
    s = "".join(out).strip("-._")
    if not s:
        s = "t3k"
    if not s.lower().endswith(".nam"):
        s = s + ".nam"
    # keep it unique-ish with size
    if size and size not in s.lower():
        s = s[:-4] + "-" + size + ".nam"
    return s[:80]


def register_names(names):
    try:
        with open(REG) as f:
            data = json.load(f)
    except Exception as e:
        print("cannot read %s: %s" % (REG, e))
        return
    irs = data.get("userIRs")
    if not isinstance(irs, list):
        print("no userIRs array")
        return
    existing = set(e.get("name") for e in irs if isinstance(e, dict))
    added = []
    for name in names:
        if name in existing:
            continue
        irs.append({"exists": True, "name": name})
        existing.add(name)
        added.append(name)
    if added:
        tmp = REG + ".tmp"
        with open(tmp, "w") as f:
            json.dump(data, f, indent=4)
        os.rename(tmp, REG)
    print("registered %d: %s" % (len(added), added))


def download_model(tok, model_url, dest):
    req = Request(
        model_url,
        headers={"Authorization": "Bearer " + tok["access_token"]},
    )
    resp = urlopen(req, timeout=120)
    tmp = dest + ".part"
    with open(tmp, "wb") as f:
        while True:
            chunk = resp.read(64 * 1024)
            if not chunk:
                break
            f.write(chunk)
    os.rename(tmp, dest)


def pick_model(models):
    """Prefer A2, then A1 feather, then A1 nano. Skip A1 standard/lite.

    TONE3000 lists A2 models with no size: one file holds its sizes (Lite and
    Full), picked on the unit through /data/nam/player.json.
    """
    ranked = []
    for m in models:
        arch = str(m.get("architecture_version") or "")
        if arch not in ALLOWED_ARCH:
            continue
        sz = (m.get("size") or "").lower()
        a2 = arch.startswith("2")
        if not a2 and sz not in ALLOWED_SIZES:
            continue
        rank = (0 if a2 else 1, 0 if sz == "feather" else 1)
        ranked.append((rank, m))
    if not ranked:
        return None
    ranked.sort(key=lambda x: x[0])
    return ranked[0][1]


def collect_tones(tok, extra_tone_id):
    tones = []
    seen = set()

    def add_page(path):
        page = 1
        while page < 20:
            payload = api_get(tok, "%s?page=%d&page_size=25" % (path, page))
            if not isinstance(payload, dict):
                break
            rows = payload.get("data") or []
            for t in rows:
                tid = t.get("id")
                if tid in seen:
                    continue
                seen.add(tid)
                tones.append(t)
            total_pages = payload.get("total_pages") or 1
            if page >= total_pages or not rows:
                break
            page += 1

    add_page("/tones/favorited")
    add_page("/tones/created")
    if extra_tone_id:
        try:
            tid = int(extra_tone_id)
        except (TypeError, ValueError):
            tid = None
        if tid and tid not in seen:
            try:
                t = api_get(tok, "/tones/%d" % tid)
                if isinstance(t, dict) and t.get("id"):
                    tones.append(t)
            except SystemExit:
                pass
    return tones


def main():
    client_id = load_key()
    tok = load_tokens()
    extra_tone = None
    if tok:
        nxt = refresh(client_id, tok)
        if nxt:
            tok = nxt
            print("refreshed TONE3000 token")
        else:
            tok = None
    if not tok:
        tok, extra_tone = oauth_lan(client_id)
        print("linked TONE3000 account")
    me = api_get(tok, "/user")
    uname = None
    if isinstance(me, dict):
        uname = me.get("username") or (me.get("user") or {}).get("username")
    print("user: %s" % (uname or "?"))

    tones = collect_tones(tok, extra_tone)
    print("tones to scan: %d" % len(tones))
    wanted_files = []
    for tone in tones:
        tid = tone.get("id")
        title = tone.get("title") or "tone"
        if tone.get("format") and tone.get("format") != "nam":
            continue
        # Without `architecture` the API returns A1 + custom only, never A2.
        models = []
        for arch in ("2", "1"):
            payload = api_get(
                tok,
                "/models?tone_id=%s&architecture=%s&page_size=50" % (tid, arch),
            )
            rows = payload.get("data") if isinstance(payload, dict) else payload
            if isinstance(rows, list):
                models.extend(rows)
        model = pick_model(models)
        if not model:
            print("skip %s: no A2 or A1 feather/nano" % title)
            continue
        ir_name = safe_name(title, model.get("name"), model.get("size"))
        dest = os.path.join(IR_DIR, ir_name + ".wav")  # file is <name>.nam.wav
        wanted_files.append((ir_name, model, dest))
        print("queue %s (%s %s)" % (ir_name, model.get("size"), model.get("architecture_version")))

    if not wanted_files:
        print("nothing to download (favorite/create tones with A2 or A1 Feather/Nano models)")
        return 0

    register_names([n for n, _m, _d in wanted_files])
    if not os.path.isdir(IR_DIR):
        os.makedirs(IR_DIR)
    for ir_name, model, dest in wanted_files:
        if os.path.isfile(dest) and os.path.getsize(dest) > 1000:
            print("exists %s" % dest)
            continue
        print("download %s" % dest)
        download_model(tok, model["model_url"], dest)
        print("  %d bytes" % os.path.getsize(dest))
    print("done. pick them in the User IR list (bypass the Fender amp block).")
    restart = True
    if "--no-restart" in sys.argv:
        restart = False
    if restart:
        print("restarting tm-stomp-server so the IR picker reloads...")
        rc = os.system("systemctl restart tm-stomp-server")
        if rc != 0:
            print("restart failed (rc=%d); run: systemctl restart tm-stomp-server" % rc)
        else:
            print("server restarted")
    else:
        print("skipped server restart; IR picker will be stale until:")
        print("  systemctl restart tm-stomp-server")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("cancelled")
        sys.exit(130)
