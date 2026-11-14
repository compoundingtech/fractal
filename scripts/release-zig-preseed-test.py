#!/usr/bin/env python3
"""Loopback-only regression tests; no Zig installation or external network."""

import contextlib
import http.server
import importlib.util
import json
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
from unittest.mock import patch
import urllib.request

spec = importlib.util.spec_from_file_location("preseed", Path(__file__).with_name("release-zig-preseed.py"))
preseed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preseed)


class Handler(http.server.BaseHTTPRequestHandler):
    paths = []
    primary_ok = False
    fallback_ok = True
    truncated = False

    def do_GET(self):
        self.paths.append(self.path)
        ok = self.primary_ok if self.path.startswith("/primary/") else self.fallback_ok
        self.send_response(200 if ok else 503)
        truncated = self.truncated if self.path.startswith("/primary/") else False
        if truncated == "length":
            self.send_header("Content-Length", "100")
        elif truncated == "chunked":
            self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        self.wfile.write(b"a\r\nshort" if truncated == "chunked" else b"archive bytes" if ok else b"unavailable")

    def log_message(self, *_):
        pass


with tempfile.TemporaryDirectory() as directory, contextlib.ExitStack() as stack:
    source = Path(directory)
    (source / "flatpak").mkdir()
    url = "https://github.com/example/package/archive/refs/tags/v1.tar.gz"
    expected = "pinned-zig-content-hash"
    manifest = source / "flatpak/zig-packages.json"
    manifest.write_text(json.dumps([{"url": url, "dest": f"vendor/p/{expected}"}]))
    server = stack.enter_context(http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    stack.callback(thread.join)
    stack.callback(server.shutdown)
    delays = []

    def open_url(candidate, **kwargs):
        if candidate == url:
            path = "/primary/archive.tar.gz"
        else:
            assert candidate == "https://codeload.github.com/example/package/tar.gz/refs/tags/v1"
            path = "/fallback/archive.tar.gz"
        return urllib.request.urlopen(f"http://127.0.0.1:{server.server_port}{path}", **kwargs)

    def fetch(archive):
        assert archive.read_bytes() == b"archive bytes"
        return expected

    options = {"open_url": open_url, "sleep": delays.append}
    preseed.seed(source, fetch=fetch, **options)
    assert delays == [1, 3]
    assert Handler.paths == ["/primary/archive.tar.gz"] * 3 + ["/fallback/archive.tar.gz"]
    print("PASS 503 retries at 1s and 3s then codeload fallback")

    Handler.paths.clear()
    Handler.primary_ok = True
    preseed.seed(source, fetch=fetch, **options)
    assert Handler.paths == ["/primary/archive.tar.gz"]
    print("PASS successful primary does not use fallback")

    for truncated in ("length", "chunked"):
        Handler.paths.clear()
        delays.clear()
        Handler.truncated = truncated
        preseed.seed(source, fetch=fetch, **options)
        assert delays == [1, 3]
        assert Handler.paths == ["/primary/archive.tar.gz"] * 3 + ["/fallback/archive.tar.gz"]
    Handler.truncated = False
    print("PASS truncated response retries then codeload fallback")

    try:
        preseed.seed(source, fetch=lambda _: "wrong-hash", **options)
    except ValueError as error:
        assert "Zig hash mismatch" in str(error)
    else:
        raise AssertionError("hash mismatch accepted")
    print("PASS Zig hash mismatch refused")

    Handler.primary_ok = False
    Handler.fallback_ok = False
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        preseed.seed(source, fetch=lambda _: (_ for _ in ()).throw(AssertionError("fetched after failure")), **options)
    assert f"WARNING Zig preseed transport unavailable for {expected}" in output.getvalue()
    print("PASS exhausted transports warn and continue")

    # Changing the manifest changes the fetch target and expected hash, without
    # changing the script. Exercise commit-style codeload conversion as well.
    url = "https://github.com/owner/repo/archive/abcdef.tar.gz"
    expected = "updated-upstream-content-hash"
    manifest.write_text(json.dumps([{"url": url, "dest": f"vendor/p/{expected}"}]))
    Handler.primary_ok = True
    preseed.seed(source, fetch=fetch, **options)
    assert preseed.codeload("https://github.com/owner/repo/archive/abcdef.tar.gz") == "https://codeload.github.com/owner/repo/tar.gz/abcdef"
    print("PASS tag and commit URLs derived from upstream pins")

    git_package = {
        "type": "git",
        "url": "https://github.com/example/unicode",
        "commit": "a" * 40,
        "dest": "vendor/p/git-content-hash",
    }
    manifest.write_text(json.dumps([git_package]))
    git_calls = []
    delays.clear()

    def git_fetch(candidate):
        assert candidate == f"git+{git_package['url']}#{git_package['commit']}"
        git_calls.append(candidate)
        if len(git_calls) < 3:
            raise subprocess.CalledProcessError(1, ["zig", "fetch", candidate])
        return "git-content-hash"

    preseed.seed(source, fetch=git_fetch, **options)
    assert len(git_calls) == 3 and delays == [1, 3]
    print("PASS Git-source record retries and verifies pinned hash without archive conversion")

    try:
        preseed.seed(source, fetch=lambda _: "wrong-git-hash", **options)
    except ValueError as error:
        assert "Zig hash mismatch" in str(error)
    else:
        raise AssertionError("Git hash mismatch accepted")
    print("PASS Git-source hash mismatch refused")

    output = io.StringIO()
    with patch.object(preseed, "run_command", side_effect=subprocess.TimeoutExpired("zig", 0.02)) as process:
        with contextlib.redirect_stdout(output):
            preseed.seed(source, attempt_timeout=0.02, **options)
        assert process.call_count == 3
        assert all(0 < call.kwargs["timeout"] <= 0.02 for call in process.call_args_list)
    assert "WARNING Zig preseed transport unavailable for git-content-hash" in output.getvalue()
    print("PASS Git subprocess deadline retries then warns")

    url = "https://deps.files.example/unicode.tar.zst"
    expected = "non-github-unicode-hash"
    manifest.write_text(json.dumps([{"type": "archive", "url": url, "dest": f"vendor/p/{expected}"}]))
    Handler.primary_ok = True
    def fetch_unicode(archive):
        assert archive.name == "unicode.tar.zst"
        return fetch(archive)
    preseed.seed(source, fetch=fetch_unicode, **options)
    print("PASS non-GitHub pins are seeded and archive suffix is preserved")

    Handler.primary_ok = False
    Handler.paths.clear()
    with contextlib.redirect_stdout(io.StringIO()):
        preseed.seed(source, fetch=fetch_unicode, **options)
    assert Handler.paths == ["/primary/archive.tar.gz"] * 3
    print("PASS non-GitHub failures retry original without codeload")

    class DripResponse:
        headers = {}
        def __enter__(self):
            return self
        def __exit__(self, *_):
            pass
        def read(self, *_):
            time.sleep(0.005)
            return b"x"

    # A trickle never trips a per-socket timeout, but must hit elapsed deadlines.
    manifest.write_text(json.dumps([{"url": url, "dest": f"vendor/p/{expected}"}] * 2))
    output = io.StringIO()
    started = time.monotonic()
    with contextlib.redirect_stdout(output):
        preseed.seed(source, fetch=lambda _: (_ for _ in ()).throw(AssertionError("accepted endless response")),
                     total_timeout=0.07, attempt_timeout=0.03,
                     sleep=lambda _: None, open_url=lambda *_args, **_kwargs: DripResponse())
    assert time.monotonic() - started < 0.5
    assert "archive attempt elapsed deadline exceeded" in output.getvalue()
    assert "WARNING Zig preseed total deadline reached" in output.getvalue()
    print("PASS drip responses obey per-attempt and total elapsed deadlines with warning")

    heartbeat = source / "grandchild-heartbeat"
    grandchild = source / "grandchild.py"
    grandchild.write_text(
        "import os, signal, sys, time\n"
        "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
        "while True:\n"
        "    with open(sys.argv[1], 'w') as output:\n"
        "        output.write(str(time.monotonic_ns()))\n"
        "    time.sleep(0.01)\n"
    )
    parent = source / "parent.py"
    parent.write_text(
        "import subprocess, sys, time\n"
        "subprocess.Popen([sys.executable, sys.argv[1], sys.argv[2]])\n"
        "time.sleep(60)\n"
    )
    started = time.monotonic()
    try:
        preseed.run_command([sys.executable, str(parent), str(grandchild), str(heartbeat)],
                            timeout=0.5, capture=True)
    except subprocess.TimeoutExpired:
        pass
    else:
        raise AssertionError("process-tree timeout was not enforced")
    assert time.monotonic() - started < 2
    assert heartbeat.exists(), "grandchild never started"
    stopped = heartbeat.read_bytes()
    time.sleep(0.1)
    assert heartbeat.read_bytes() == stopped, "grandchild survived group timeout cleanup"
    assert preseed.run_command([sys.executable, "-c", "print('normal output')"],
                               timeout=2, capture=True) == "normal output\n"
    print("PASS timed-out process group kills TERM-resistant grandchild and reaps parent")
