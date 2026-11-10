#!/usr/bin/env python3
"""Unit tests use terminal-cell text, not client result reports."""
import hashlib
import unittest
from client_control_capture import verify


class TerminalCaptureTest(unittest.TestCase):
    def test_seeded_title_and_body_digest(self):
        digest = hashlib.sha256(b"First line\nSecond line").hexdigest()
        result = verify("title: client-control fixture 0001\n| First line   |\n| Second line  |\n",
                        "client-control fixture 0001", digest, 2, 3, 3, 15)
        self.assertEqual(result["independent_check"], "passed")
        self.assertEqual(result["body_sha256"], digest)

    def test_missing_title_wrong_body_and_raw_escape_fail(self):
        for capture in ["wrong title\nbody\n", "seeded title\nwrong body\n",
                        "seeded title\nbody\x1b[2J\n"]:
            with self.subTest(capture=capture), self.assertRaises(ValueError):
                verify(capture, "seeded title", hashlib.sha256(b"body").hexdigest(), 2, 2, 1, 20)

    def test_truncated_body_and_outside_rectangle_fail(self):
        with self.assertRaises(ValueError):
            verify("seeded title\nfirst line\n", "seeded title",
                   hashlib.sha256(b"first line\nmissing line").hexdigest(), 2, 2, 1, 20)
        with self.assertRaises(ValueError):
            verify("seeded title\nbody\n", "seeded title", "0" * 64, 2, 3, 1, 20)


if __name__ == "__main__":
    unittest.main()
