"""Pure source controls for st3-instance-pair-proof; no native fixture is started.

This file is review input only. Do not run before the exact-script testing review.
"""

import contextlib
import hashlib
import importlib.util
from importlib.machinery import SourceFileLoader
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("st3-instance-pair-proof")
# The source has no .py suffix, so use a source loader explicitly.
SPEC = importlib.util.spec_from_loader("instance_pair_proof", SourceFileLoader("instance_pair_proof", str(SCRIPT)))
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)


class SourceControls(unittest.TestCase):
    def rows(self):
        return [dict(name=proof.PREFIX + f"{number:02d}",
                     hash=hashlib.sha256(proof.document_bytes(number)).hexdigest(),
                     size=len(proof.document_bytes(number)), latest=True,
                     binding_claim_id=f"claim-{number}")
                for number in range(50)]

    def test_exact_fifty_and_content_are_required(self):
        rows = self.rows()
        self.assertEqual(len(proof.compare_documents(rows)), 50)
        for changed, code in ((rows[:-1], "document-count"),
                              (rows[:-1] + [rows[0]], "duplicate-document")):
            with self.assertRaisesRegex(proof.Refused, code):
                proof.compare_documents(changed)
        wrong = self.rows()
        wrong[1]["hash"] = "0" * 64
        with self.assertRaisesRegex(proof.Refused, "document-hash"):
            proof.compare_documents(wrong)

    def test_binding_checks_id_body_and_physical_cut(self):
        row = self.rows()[0] | {"created_at_unix_ms": 42, "created_index": 9, "owner": "node-a"}
        claim = {"id": row["binding_claim_id"], "kind": "doc.bound", "subject": row["name"],
                 "body": {key: row[key] for key in ("name", "hash", "size")},
                 "accepted_at_unix_ms": 42, "store_index": 9, "actor": None, "origin": "node-a"}
        proof.verify_binding(row, [claim])
        with self.assertRaisesRegex(proof.Refused, "binding-physical-index-mismatch"):
            proof.verify_binding(row, [claim | {"store_index": 10}])

    def test_different_input_has_bounded_secret_free_claim_list(self):
        basic = {key: "equal" for key in proof.COMPARE_FIELDS}
        basic["fleet_id"] = "fleet/sample"
        basic["member_key_fingerprints"] = {"a": "first", "b": "second"}
        basic["_claim_refs"] = [(f"{number:064x}", "doc.bound", "left") for number in range(30)]
        other = dict(basic, input_sha256="changed",
                     _claim_refs=[(f"{number:064x}", "doc.bound", "right") for number in range(30)])
        with self.assertRaises(proof.CutMismatch) as raised:
            proof.compare_cuts(basic, other)
        self.assertEqual(len(raised.exception.differences), 20)
        self.assertEqual(set(raised.exception.differences[0]), {"id", "a_kind", "b_kind"})

    def test_errors_and_child_environment_do_not_expose_tokens(self):
        self.assertEqual(proof.safe_error(proof.Refused("secret-token-value")), "fixture-refused")
        self.assertEqual(proof.safe_error(proof.Refused("wrong-binary")), "wrong-binary")
        child = proof.Member("a", "person/test").environment()
        self.assertEqual(set(child), {"HOME", "PATH", "LANG", "TMPDIR", "PYTHONDONTWRITEBYTECODE"})
        self.assertNotIn("ST_AGENT", child)

    def test_generated_fleet_file_must_be_loopback_only(self):
        with tempfile.TemporaryDirectory() as parent:
            member = proof.Member("b", "person/test")
            member.fleet_file = Path(parent) / "fleet.toml"
            member.fleet_file.write_text(
                'transports = ["loopback"]\n'
                'sponsor_routes = ["http://127.0.0.1:31313"]\n'
            )
            member.check_loopback_transport()
            member.fleet_file.write_text(member.fleet_file.read_text() + 'extra = "fabric://node"\n')
            with self.assertRaisesRegex(proof.Refused, "nonloopback-route"):
                member.check_loopback_transport()

    def test_unreaped_child_preserves_root_and_fails_receipt(self):
        with tempfile.TemporaryDirectory() as parent:
            root = Path(parent) / "fixture"
            output = io.StringIO()
            with mock.patch.object(proof, "ROOT", root), mock.patch.object(proof, "preflight"), \
                    mock.patch.object(proof.Member, "prepare", side_effect=RuntimeError("secret-token-value")), \
                    mock.patch.object(proof.Member, "stop", return_value=False), \
                    contextlib.redirect_stdout(output):
                self.assertEqual(proof.main(), 1)
            receipt = json.loads(output.getvalue())
            self.assertFalse(receipt["accepted"])
            self.assertFalse(receipt["children_reaped"])
            self.assertFalse(receipt["owned_root_removed"])
            self.assertEqual(receipt["cleanup_error"], "owned-cleanup-failed")
            self.assertNotIn("secret-token-value", output.getvalue())
            self.assertTrue(root.exists())


if __name__ == "__main__":
    unittest.main()
