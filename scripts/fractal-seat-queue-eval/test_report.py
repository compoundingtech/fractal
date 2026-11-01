import runpy
from pathlib import Path
import unittest

work_tag = runpy.run_path(str(Path(__file__).with_name("report")))["work_tag"]


class WorkTagTests(unittest.TestCase):
    def test_prefixes_and_conflict(self):
        old = "st3-work:old@1@2@inc"
        new = "fractal-work:new@9@8@other"
        self.assertEqual(work_tag([old]), old)
        self.assertEqual(work_tag([new]), new)
        self.assertEqual(work_tag([new, old]), old)
        self.assertEqual(work_tag([old, new]), old)
        self.assertEqual(work_tag(["fractal-work-handoff:x"]), "")


if __name__ == "__main__":
    unittest.main()
