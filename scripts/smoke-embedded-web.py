#!/usr/bin/env python3
"""Exercise the embedded browser boundary against built web assets and an isolated daemon."""
import argparse
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
from urllib.parse import urlsplit

parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("--dist", type=Path, required=True)
args = parser.parse_args()
with tempfile.TemporaryDirectory(prefix="fractal-embedded-web-") as temporary:
    root = Path(temporary)
    state = root / "state"
    state.mkdir(mode=0o700)
    dist = args.dist.resolve()
    config = root / "config.toml"
    config.write_text('person = "person/robin"\n')
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    environment = {key: value for key, value in os.environ.items()
                   if key not in {"ST3_ENDPOINT", "FRACTAL_ENDPOINT", "ST_INSTANCE"}}
    log = root / "daemon.log"
    with log.open("w+") as output:
        daemon = subprocess.Popen([
            str(args.binary.resolve()), "up", "--config", str(config),
            "--state-dir", str(state), "--socket", str(root / "api.sock"),
            "--client-gateway-socket", str(root / "client.sock"),
            "--web-listen", f"127.0.0.1:{port}", "--web-dist", str(dist),
        ], env=environment, stdout=output, stderr=output)
        try:
            launch = state / "web-launch-url"
            deadline = time.monotonic() + 90
            while not launch.exists():
                if daemon.poll() is not None:
                    raise RuntimeError(f"daemon exited {daemon.returncode}: {log.read_text()}")
                if time.monotonic() >= deadline:
                    raise RuntimeError(f"daemon startup timed out: {log.read_text()}")
                time.sleep(0.1)
            def request(path, cookie=None, host=None, method="GET", origin=None):
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
                headers = {"Host": host or f"127.0.0.1:{port}"}
                if cookie:
                    headers["Cookie"] = cookie
                if origin:
                    headers["Origin"] = origin
                connection.request(method, path, headers=headers)
                response = connection.getresponse()
                result = response.status, dict(response.getheaders()), response.read()
                connection.close()
                return result
            assert request("/")[0] == 401
            initial = launch.read_text()
            parsed = urlsplit(initial)
            status, headers, _ = request(parsed.path + "?" + parsed.query)
            assert status == 303, (status, headers)
            assert "HttpOnly; SameSite=Strict" in headers["set-cookie"]
            assert launch.read_text() != initial
            cookie = headers["set-cookie"].split(";")[0]
            assert request(parsed.path + "?" + parsed.query)[0] == 403
            status, headers, body = request("/", cookie)
            assert status == 200 and body == (dist / "index.html").read_bytes()
            assert "content-security-policy" in headers
            assert request("/nested/spa", cookie)[2] == body
            assert request("/build-identity.json", cookie)[0] == 200
            assert request("/", cookie, host=f"evil.example:{port}")[0] == 403
            assert request("/v1/schema", cookie)[0] == 403
            assert request("/v1/client/actions", cookie, method="POST", origin="http://evil.example")[0] == 403
            status, _, capabilities = request("/v1/client/capabilities", cookie)
            assert status == 200, (status, capabilities)
            json.loads(capabilities)
            print(json.dumps({"result": "pass", "asset_kind": "built-dist",
                              "checks": ["index", "SPA", "build-identity", "single-use-token", "cookie", "host", "origin", "client-GET", "admin-denied"]}))
        finally:
            daemon.terminate()
            try:
                daemon.wait(timeout=15)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait()
