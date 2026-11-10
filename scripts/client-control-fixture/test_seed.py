import copy
import importlib.util
from pathlib import Path
import socket
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('fixture_seed', Path(__file__).with_name('seed.py'))
seed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(seed)


class FixtureSeedTests(unittest.TestCase):
    def test_plain_text_rules(self):
        for text in [seed.TITLE, seed.BODY, 'a' * 60, 'One plain paragraph']:
            self.assertEqual(seed.validate_text(text), text)
        for text in ['', 'a' * 61, ' leading', 'trailing ', 'two  spaces', 'two\nlines',
                     'tab\there', 'nonascii é', '\x7f', '{}', '123', 'true', '"JSON"']:
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, 'fixture-text-refused'):
                seed.validate_text(text)
        for char in '*_#[]`>|':
            with self.subTest(char=char), self.assertRaises(ValueError):
                seed.validate_text(f'No {char} syntax')

    def test_exact_own_operator_socket_only(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            own = home / '.st-instance' / seed.INSTANCE / 'run/fractal.sock'
            own.parent.mkdir(parents=True)
            with socket.socket(socket.AF_UNIX) as listener:
                listener.bind(str(own))
                self.assertEqual(seed.operator_socket(own, home), own)
                for candidate in [home / 'run/fractal.sock', home / 'run/fractal-client.sock',
                                  home / '.st-instance/other/run/fractal.sock',
                                  own.with_name('fractal-client.sock'), home / 'outside.sock']:
                    with self.subTest(path=candidate), self.assertRaisesRegex(ValueError, 'fixture-path-refused'):
                        seed.operator_socket(candidate, home)
                alias = own.with_name('alias.sock')
                alias.symlink_to(own)
                with self.assertRaises(ValueError):
                    seed.operator_socket(alias, home)
            own.unlink()
            other = home / 'outside'
            other.mkdir()
            own.symlink_to(other / 'normal.sock')
            with self.assertRaises(ValueError):
                seed.operator_socket(own, home)
            own.unlink()
            own.parent.rmdir()
            own.parent.symlink_to(other)
            with self.assertRaises(ValueError):
                seed.operator_socket(own, home)

    def test_missing_socket_never_creates_or_launches_anything(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(seed.subprocess, 'run') as run:
            home = Path(temporary)
            own = home / '.st-instance' / seed.INSTANCE / 'run/fractal.sock'
            with patch.dict(seed.os.environ, {'HOME': str(home)}), self.assertRaises(FileNotFoundError):
                seed.Fixture('fractal', own).attention()
            run.assert_not_called()
            self.assertFalse(own.parent.exists())

    def test_root_receipt_matches_rust_bytes_without_newline_normalization(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            migrated = home / '.fractal-instance' / seed.INSTANCE
            migrated.mkdir(parents=True)
            receipt = migrated / '.fractal-migration-completed'
            receipt.write_bytes(seed.MIGRATION.replace('\n', '\r\n').encode('ascii'))
            self.assertEqual(seed.instance_root(home), home / '.st-instance' / seed.INSTANCE)
            receipt.write_bytes(seed.MIGRATION.encode('ascii'))
            self.assertEqual(seed.instance_root(home), migrated)

    def test_seed_refuses_existing_read_position_before_ask_creation(self):
        with patch.object(seed, 'Fixture') as fixture_type:
            fixture = fixture_type.return_value
            fixture.attention.return_value = []
            fixture.claims.return_value = [{'id': 'claim/position',
                                           'kind': 'custom.mailbox.read-position'}]
            with self.assertRaisesRegex(ValueError, 'fixture-read-position-refused'):
                seed.seed(fixture, 'fixture-key', seed.TITLE, seed.BODY)
            fixture.cli.assert_not_called()

    def evidence(self):
        ask = 'step-run/fixture/ask'
        items = [{'id': 'attention/fixture', 'source_id': ask, 'person_id': seed.PERSON,
                  'attention_kind': 'person-step', 'state': 'open', 'title': seed.TITLE,
                  'presentation': {'kind': 'request'}, 'request': {'question': seed.BODY}}]
        claims = [{'subject': ask, 'id': 'claim/request', 'kind': 'work.person-asked',
                   'actor': seed.ACTOR, 'body': {'fields': {'person': seed.PERSON,
                   'title': seed.TITLE, 'request': {'question': seed.BODY}}}}]
        return ask, items, claims

    def test_open_request_receipt_names_claims_without_inventing_unread(self):
        ask, items, claims = self.evidence()
        receipt = seed.no_read_mutation(items, claims, ask)
        self.assertEqual(receipt['check'], 'no-read-mutation')
        self.assertEqual(receipt['claim_ids'], ['claim/request'])
        self.assertEqual(receipt['unread_bit'], 'not-supported-for-native-asks')
        self.assertEqual(receipt['body_sha256'], seed.hashlib.sha256(seed.BODY.encode()).hexdigest())

    def test_answer_read_ack_close_and_position_records_are_refused(self):
        ask, items, claims = self.evidence()
        for kind in ['message.read', 'work.acknowledged', 'work.person-done',
                     'work.person-cancelled', 'message.closed', 'custom.mailbox.read-position']:
            mutated = claims + [{'subject': ask, 'id': 'claim/mutation', 'kind': kind}]
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                seed.no_read_mutation(items, mutated, ask)
        closed = copy.deepcopy(items)
        closed[0]['state'] = 'resolved'
        with self.assertRaises(ValueError):
            seed.no_read_mutation(closed, claims, ask)
        with self.assertRaises(ValueError):
            seed.no_read_mutation(items * 2, claims, ask)


if __name__ == '__main__':
    unittest.main()
