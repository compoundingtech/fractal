#!/usr/bin/env python3
"""Check QA launcher aliases without starting a PTY or contacting a daemon."""
import os
import unittest
from unittest.mock import patch

with patch.dict(os.environ, {"PTY_BIN": "fixture-pty"}):
    import interaction_qa as qa


class PersonAliases(unittest.TestCase):
    def test_accepted_aliases_keep_legacy_child_spelling(self):
        for preferred, legacy, expected in [
            ("person/new", None, "person/new"),
            (None, "person/old", "person/old"),
            ("person/equal", "person/equal", "person/equal"),
            ("", "person/old", "person/old"),
            ("person/new", "", "person/new"),
            (None, None, "person/alex"),
        ]:
            environment = {}
            if preferred is not None:
                environment["FRACTAL_PERSON"] = preferred
            if legacy is not None:
                environment["ST3_PERSON"] = legacy
            with self.subTest(preferred=preferred, legacy=legacy), patch.dict(
                os.environ, environment, clear=True
            ), patch.object(qa.subprocess, "run") as run, patch.object(qa, "wait_screen"):
                qa.start("fixture-binary", "fixture")
                self.assertIn(f"ST3_PERSON={expected}", run.call_args.args[0])

    def test_conflict_fails_before_launch_without_exposing_values(self):
        with patch.dict(os.environ, {
            "FRACTAL_PERSON": "person/preferred-secret",
            "ST3_PERSON": "person/legacy-secret",
        }, clear=True), patch.object(qa.subprocess, "run") as run:
            with self.assertRaises(ValueError) as caught:
                qa.start("fixture-binary", "fixture")
            message = str(caught.exception)
            self.assertIn("FRACTAL_PERSON and ST3_PERSON", message)
            self.assertNotIn("secret", message)
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
