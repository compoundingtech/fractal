#!/usr/bin/env python3
"""Loopback-only regression tests; no Zig installation or external network."""

import contextlib
import http.server
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import urllib.request

spec = importlib.util.spec_from_file_location("preseed", Path(__file__).with_name("release-zig-preseed.py"))
preseed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preseed)


class Handler(http.server.BaseHTTPRequestHandler):
    paths = []
    primary_ok = False
    fallback_ok = True

    def do_GET(self):
        self.paths.append(self.path)
        ok = self.primary_ok if self.path.startswith("/primary/") else self.fallback_ok
        self.send_response(200 if ok else 503)
        self.end_headers()
        self.wfile.write(b"archive bytes" if ok else b"unavailable")

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

    try:
        preseed.seed(source, fetch=lambda _: "wrong-hash", **options)
    except ValueError as error:
        assert "Zig hash mismatch" in str(error)
    else:
        raise AssertionError("hash mismatch accepted")
    print("PASS Zig hash mismatch refused")

    Handler.primary_ok = False
    Handler.fallback_ok = False
    try:
        preseed.seed(source, fetch=lambda _: (_ for _ in ()).throw(AssertionError("fetched after failure")), **options)
    except urllib.error.HTTPError:
        pass
    else:
        raise AssertionError("exhausted transports accepted")
    print("PASS exhausted transports fail closed")

    # Changing the manifest changes the fetch target and expected hash, without
    # changing the script. Exercise commit-style codeload conversion as well.
    url = "https://github.com/owner/repo/archive/abcdef.tar.gz"
    expected = "updated-upstream-content-hash"
    manifest.write_text(json.dumps([{"url": url, "dest": f"vendor/p/{expected}"}]))
    Handler.primary_ok = True
    preseed.seed(source, fetch=fetch, **options)
    assert preseed.codeload("https://github.com/owner/repo/archive/abcdef.tar.gz") == "https://codeload.github.com/owner/repo/tar.gz/abcdef"
    print("PASS tag and commit URLs derived from upstream pins")
