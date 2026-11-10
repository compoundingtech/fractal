#!/usr/bin/env python3
"""Verify an independently captured terminal pane, never a device.result payload.

Capture outside the TUI process with `tmux capture-pane -p -J -t PANE` and save
stdout. Supply the visible body rectangle (one-based rows/columns, inclusive).
Use a terminal wide/tall enough to show the entire seeded title and body.
"""
import argparse
import hashlib
import json
from pathlib import Path


def verify(capture, title, body_sha256, first_row, last_row, first_column, last_column):
    if "\x1b" in capture or "\x00" in capture:
        raise ValueError("capture must contain rendered plain terminal cells")
    rows = capture.splitlines()
    if not (1 <= first_row <= last_row <= len(rows) and 1 <= first_column <= last_column):
        raise ValueError("body rectangle is outside the captured terminal")
    if not any(title in row for row in rows):
        raise ValueError("seeded title is not rendered")
    body = "\n".join(row[first_column - 1:last_column].rstrip()
                     for row in rows[first_row - 1:last_row])
    digest = hashlib.sha256(body.encode("utf-8")).hexdigest()
    if digest != body_sha256:
        raise ValueError(f"rendered body digest mismatch: {digest}")
    return {"independent_check": "passed", "title": title, "body_sha256": digest,
            "capture_sha256": hashlib.sha256(capture.encode("utf-8")).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--title", required=True)
    parser.add_argument("--body-sha256", required=True)
    parser.add_argument("--body-rectangle", required=True, nargs=4, type=int,
                        metavar=("FIRST_ROW", "LAST_ROW", "FIRST_COLUMN", "LAST_COLUMN"))
    args = parser.parse_args()
    print(json.dumps(verify(args.capture.read_text(), args.title, args.body_sha256,
                            *args.body_rectangle), sort_keys=True))


if __name__ == "__main__":
    main()
