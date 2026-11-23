"""The tool's own tests, its manifest's check: the executable run as the host runs it, one handed object on
standard input and one answer read off standard output."""

import json
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def handed(kind="call", **fields):
    """A handed object as the host writes it, under a record holding the entry's capability."""
    return {
        "protocol": 1,
        "kind": kind,
        "session": {
            "holderName": "tester",
            "namespaces": ["example"],
            "capabilities": ["example.read"],
            "grants": [],
        },
        **fields,
    }


def run(asked):
    """The executable run once in its folder: its exit code and what it wrote on standard output."""
    done = subprocess.run(
        [sys.executable, "count.py"],
        cwd=HERE,
        input=json.dumps(asked).encode(),
        capture_output=True,
        check=False,
    )
    return done.returncode, done.stdout.decode()


class CountWords(unittest.TestCase):
    def test_a_call_is_answered_with_the_count_as_text_and_as_an_object(self):
        code, said = run(handed(entry="example/count_words_py", arguments={"text": "one two  three\n"}))
        answer = json.loads(said)

        self.assertEqual(code, 0)
        self.assertFalse(answer["isError"])
        self.assertEqual(answer["structuredContent"], {"words": 3, "characters": 15})
        self.assertEqual(json.loads(answer["content"][0]["text"]), answer["structuredContent"])

    def test_characters_are_code_points(self):
        _, said = run(handed(entry="example/count_words_py", arguments={"text": "café au lait"}))

        self.assertEqual(json.loads(said)["structuredContent"], {"words": 3, "characters": 12})

    def test_an_entry_it_does_not_hold_is_answered_as_a_failure(self):
        code, said = run(handed(entry="example/other", arguments={}))
        answer = json.loads(said)

        self.assertEqual(code, 0)
        self.assertTrue(answer["isError"])
        self.assertIn("ERROR: ", answer["content"][0]["text"])

    def test_an_object_that_does_not_read_ends_unclean(self):
        done = subprocess.run(
            [sys.executable, "count.py"], cwd=HERE, input=b"not json", capture_output=True, check=False
        )

        self.assertNotEqual(done.returncode, 0)
        self.assertIn(b"the call does not read", done.stderr)


if __name__ == "__main__":
    unittest.main()
