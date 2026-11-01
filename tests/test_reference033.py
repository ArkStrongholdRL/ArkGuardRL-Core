"""Development oracle checks; explicitly opt in with ARKGUARD_REFERENCE_ROOT."""
import json
import os
from pathlib import Path
import subprocess
import unittest

ENGINE = os.environ.get('ARKGUARD_REFERENCE_ROOT')
BRIDGE = Path(__file__).resolve().parents[1] / 'tools/reference/session.mjs'


@unittest.skipUnless(ENGINE, 'Set ARKGUARD_REFERENCE_ROOT to the 0.3.3 reference checkout')
class ReferenceTests(unittest.TestCase):
    def request(self, commands):
        result = subprocess.run(['node', str(BRIDGE), ENGINE],
            input=''.join(json.dumps(c) + '\n' for c in commands),
            text=True, capture_output=True, timeout=20, check=True)
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(len(replies), len(commands), result.stderr)
        return replies

    def test_all_difficulties_and_seed_reuse(self):
        commands = []
        for difficulty in ('FUNNY', 'NORMAL', 'HARD', 'ABYSS'):
            for _ in range(2):
                commands.extend([{'cmd': 'reset', 'seed': 42, 'difficulty': difficulty},
                    {'cmd': 'step', 'action': {'t': 'g.infoReady'}}])
        commands.append({'cmd': 'close'})
        replies = self.request(commands)
        for reply in replies:
            self.assertTrue(reply['ok'], reply)
        for i in range(0, 16, 4):
            self.assertEqual(replies[i]['state'], replies[i + 2]['state'])
            self.assertEqual(replies[i + 1]['state'], replies[i + 3]['state'])
            self.assertEqual(replies[i]['state']['phase'], 'INFO_CHECK')
            self.assertEqual(replies[i + 1]['state']['phase'], 'BAND_DRAFT')
            self.assertTrue(replies[i + 1]['accepted'])
            self.assertFalse(replies[i + 1]['truncated'])

    def test_bad_budget_fails_closed(self):
        replies = self.request([{'cmd': 'reset', 'seed': 42},
            {'cmd': 'step', 'action': {'t': 'g.infoReady'}, 'maxCallbacks': 0}])
        self.assertTrue(replies[0]['ok'])
        self.assertFalse(replies[1]['ok'])
        self.assertEqual(replies[1]['error'], 'Invalid callback budget')


if __name__ == '__main__':
    unittest.main()
