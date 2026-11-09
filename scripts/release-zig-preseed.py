#!/usr/bin/env python3
"""Preseed Ghostty's pinned package manifest without changing dependency content."""

import argparse
from contextlib import contextmanager
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import urllib.parse


ARCHIVE = re.compile(r"https://github\.com/([^/]+/[^/]+)/archive/(.+)\.tar\.gz")


def codeload(url):
    match = ARCHIVE.fullmatch(url)
    if not match:
        raise ValueError(f"unsupported GitHub archive URL: {url}")
    return f"https://codeload.github.com/{match[1]}/tar.gz/{match[2]}"


class Budget:
    def __init__(self, total=660, attempt=60):
        self.deadline = time.monotonic() + total
        self.attempt = attempt

    def remaining(self):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("Zig preseed total deadline exceeded")
        return min(self.attempt, remaining)

    def pause(self, delay, sleep):
        if delay:
            sleep(min(delay, self.remaining()))
        self.remaining()


@contextmanager
def elapsed_deadline(seconds):
    # POSIX runners only. Unlike socket timeouts this also stops a peer that
    # keeps sending tiny chunks forever.
    def expired(*_):
        raise TimeoutError("archive attempt elapsed deadline exceeded")
    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def download(url, destination, sleep=time.sleep, open_url=urllib.request.urlopen, budget=None):
    budget = budget or Budget()
    urls = [(url, 0), (url, 1), (url, 3)]
    if ARCHIVE.fullmatch(url):
        urls.append((codeload(url), 0))
    for index, (candidate, delay) in enumerate(urls):
        budget.pause(delay, sleep)
        try:
            with elapsed_deadline(budget.remaining()), open_url(candidate, timeout=budget.remaining()) as response, destination.open("wb") as output:
                shutil.copyfileobj(response, output)
                length = response.headers.get("Content-Length")
                if length is not None and output.tell() != int(length):
                    raise urllib.error.ContentTooShortError("truncated archive response", None)
            return
        except (OSError, urllib.error.URLError, http.client.HTTPException) as error:
            if index == len(urls) - 1:
                raise
            print(f"Archive transport failed: {candidate}: {error}", flush=True)


def fetch_git(package, fetch, sleep=time.sleep, budget=None):
    budget = budget or Budget()
    commit = package["commit"]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError(f"invalid Git commit pin: {commit}")
    url = f"git+{package['url']}#{commit}"
    for attempt, delay in enumerate((0, 1, 3)):
        budget.pause(delay, sleep)
        try:
            return fetch(url)
        except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            if attempt == 2:
                raise
            print(f"Git transport failed: {url}: {error}", flush=True)


def run_command(args, *, timeout, capture=False):
    # Git/Zig can launch helpers that inherit stdout and outlive their parent.
    # Own their session so timeout cleanup covers the complete process tree.
    process = subprocess.Popen(
        args, start_new_session=True, text=True,
        stdout=subprocess.PIPE if capture else None,
    )
    try:
        output, _ = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        def kill_group(sig):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                pass
        kill_group(signal.SIGTERM)
        try:
            process.communicate(timeout=0.2)
        except subprocess.TimeoutExpired:
            pass
        finally:
            # Even if the direct child already exited, helpers may ignore TERM.
            kill_group(signal.SIGKILL)
            process.communicate()
        raise
    if process.returncode:
        raise subprocess.CalledProcessError(process.returncode, args, output)
    return output


def seed(source, *, fetch=None, total_timeout=660, attempt_timeout=60, **download_options):
    budget = Budget(total_timeout, attempt_timeout)
    # This upstream generated manifest includes transitive pins (notably zigimg)
    # as well as the direct build.zig.zon pins. No release-owned URL/hash list.
    packages = json.loads((source / "flatpak/zig-packages.json").read_text())
    pins = packages
    if not pins:
        raise ValueError("Ghostty package manifest is empty")
    if fetch is None:
        def fetch(archive):
            return run_command(["zig", "fetch", str(archive)], capture=True, timeout=budget.remaining()).strip()
    with tempfile.TemporaryDirectory(prefix="zig-preseed-") as directory:
        directory = Path(directory)
        for package in pins:
            try:
                budget.remaining()
            except TimeoutError:
                print("WARNING Zig preseed total deadline reached; remaining packages left to zig build", flush=True)
                break
            expected = package["dest"].removeprefix("vendor/p/")
            if package["dest"] != f"vendor/p/{expected}" or "/" in expected or not expected:
                raise ValueError(f"invalid Zig cache pin: {package['dest']}")
            try:
                if package.get("type") == "git":
                    # Preserve Git package semantics; no tarball substitute.
                    actual = fetch_git(package, fetch, download_options.get("sleep", time.sleep), budget)
                else:
                    archive = directory / Path(urllib.parse.urlsplit(package["url"]).path).name
                    download(package["url"], archive, budget=budget, **download_options)
                    actual = fetch(archive)
            except (OSError, urllib.error.URLError, http.client.HTTPException,
                    subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
                print(f"WARNING Zig preseed transport unavailable for {expected}: {error}; left to zig build", flush=True)
                continue
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
    try:
        run_command(["git", "clone", "--filter=blob:none", "--no-checkout", repo, str(source)], timeout=120)
        run_command(["git", "-C", str(source), "checkout", "--detach", commit], timeout=120)
    except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f"WARNING Ghostty preseed checkout unavailable: {error}; left to Cargo", flush=True)
        return
    # No --global-cache-dir: the Cargo build uses this same default Zig cache.
    seed(source)
    with args.github_env.open("a") as output:
        output.write(f"GHOSTTY_SOURCE_DIR={source}\n")


if __name__ == "__main__":
    main()
