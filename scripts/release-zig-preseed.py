#!/usr/bin/env python3
"""Preseed GitHub archives from Ghostty's pinned package manifest, not new pins."""

import argparse
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


ARCHIVE = re.compile(r"https://github\.com/([^/]+/[^/]+)/archive/(.+)\.tar\.gz")


def codeload(url):
    match = ARCHIVE.fullmatch(url)
    if not match:
        raise ValueError(f"unsupported GitHub archive URL: {url}")
    return f"https://codeload.github.com/{match[1]}/tar.gz/{match[2]}"


def download(url, destination, sleep=time.sleep, open_url=urllib.request.urlopen):
    # Three bounded attempts on the original transport; one on codeload.
    urls = [(url, 0), (url, 1), (url, 3), (codeload(url), 0)]
    for index, (candidate, delay) in enumerate(urls):
        if delay:
            sleep(delay)
        try:
            with open_url(candidate, timeout=60) as response, destination.open("wb") as output:
                shutil.copyfileobj(response, output)
                length = response.headers.get("Content-Length")
                if length is not None and output.tell() != int(length):
                    raise urllib.error.ContentTooShortError("truncated archive response", None)
            return
        except (OSError, urllib.error.URLError, http.client.HTTPException) as error:
            if index == len(urls) - 1:
                raise
            print(f"Archive transport failed: {candidate}: {error}", flush=True)


def fetch_git(package, fetch, sleep=time.sleep):
    commit = package["commit"]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError(f"invalid Git commit pin: {commit}")
    url = f"git+{package['url']}#{commit}"
    for attempt, delay in enumerate((0, 1, 3)):
        if delay:
            sleep(delay)
        try:
            return fetch(url)
        except (OSError, subprocess.CalledProcessError) as error:
            if attempt == 2:
                raise
            print(f"Git transport failed: {url}: {error}", flush=True)


def seed(source, *, fetch=None, **download_options):
    # This upstream generated manifest includes transitive pins (notably zigimg)
    # as well as the direct build.zig.zon pins. No release-owned URL/hash list.
    packages = json.loads((source / "flatpak/zig-packages.json").read_text())
    pins = [package for package in packages if package.get("url", "").startswith("https://github.com/")]
    if not pins:
        raise ValueError("Ghostty package manifest contains no GitHub archive pins")
    if fetch is None:
        def fetch(archive):
            return subprocess.check_output(["zig", "fetch", str(archive)], text=True).strip()
    with tempfile.TemporaryDirectory(prefix="zig-preseed-") as directory:
        archive = Path(directory) / "package.tar.gz"
        for package in pins:
            expected = package["dest"].removeprefix("vendor/p/")
            if package["dest"] != f"vendor/p/{expected}" or "/" in expected or not expected:
                raise ValueError(f"invalid Zig cache pin: {package['dest']}")
            if package.get("type") == "git":
                # vaxis requires its own uucode Git pin in addition to Ghostty's
                # newer archive pin. Preserve Git package semantics; no tarball substitute.
                actual = fetch_git(package, fetch, download_options.get("sleep", time.sleep))
            else:
                download(package["url"], archive, **download_options)
                actual = fetch(archive)
            if actual != expected:
                raise ValueError(f"Zig hash mismatch for {package['url']}: expected {expected}, got {actual}")
            print(f"PASS Zig package hash verified: {expected}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--github-env", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    build = (root / "vendor/libghostty-vt-sys/build.rs").read_text()
    repo = re.search(r'const GHOSTTY_REPO: &str = "([^"]+)";', build)[1]
    commit = re.search(r'const GHOSTTY_COMMIT: &str = "([0-9a-f]{40})";', build)[1]
    source = Path(tempfile.mkdtemp(prefix="release-ghostty-", dir=os.environ["RUNNER_TEMP"]))
    subprocess.run(["git", "clone", "--filter=blob:none", "--no-checkout", repo, str(source)], check=True)
    subprocess.run(["git", "-C", str(source), "checkout", "--detach", commit], check=True)
    # No --global-cache-dir: the Cargo build uses this same default Zig cache.
    seed(source)
    with args.github_env.open("a") as output:
        output.write(f"GHOSTTY_SOURCE_DIR={source}\n")


if __name__ == "__main__":
    main()
