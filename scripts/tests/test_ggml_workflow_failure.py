#!/usr/bin/env python3
"""Execute each workflow's actual ggml environment setup under CI's bash flags."""
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class GgmlWorkflowFailureTests(unittest.TestCase):
    def setup_script(self, workflow):
        lines = (ROOT / '.github/workflows' / workflow).read_text().splitlines()
        index = next(i for i, line in enumerate(lines) if 'bun scripts/ggml-stack.ts' in line and '--github-env' in line)
        setup = '\n'.join(line.strip() for line in lines[index:index + 2])
        # Substitute only runner expressions, retaining the production shell code.
        return re.sub(r'\$\{\{.*?\}\}', 'x86_64-unknown-linux-gnu', setup)

    def test_builder_failure_stops_the_job_before_cargo(self):
        for workflow in ('build.yml', 'release.yml'):
            with self.subTest(workflow=workflow):
                script = "bun() { printf 'GGML_LIB_DIR=/partial/build\\n'; return 37; }\n"
                script += self.setup_script(workflow) + "\nprintf 'CARGO_STARTED\\n'\n"
                result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', script], text=True, capture_output=True)
                self.assertEqual(result.returncode, 37, result.stderr)
                self.assertNotIn('CARGO_STARTED', result.stdout)

    def test_success_exports_exact_values_to_the_build(self):
        for workflow in ('build.yml', 'release.yml'):
            with self.subTest(workflow=workflow):
                script = "bun() { printf 'GGML_LIB_DIR=/tmp/native libraries\\nGGML_BACKEND_DL=1\\n'; }\n"
                script += self.setup_script(workflow) + "\nprintf '%s|%s' \"$GGML_LIB_DIR\" \"$GGML_BACKEND_DL\"\n"
                result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', script], text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, '/tmp/native libraries|1')


if __name__ == '__main__':
    unittest.main()
