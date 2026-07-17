import copy
import hashlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from verify import REQUIRED_CASES, validate
from make_payload import payload_bytes
from make_playbook import render_playbook


def valid_manifest():
    source = Path("tools/fixture-project/chaos/chaos.yml")
    parity = json.loads(Path("tools/oracle/parity/control-flow.json").read_text())
    return {
        "schema_version": 1,
        "target": "<fixture>",
        "fixture_source": str(source),
        "fixture_source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "generated_playbook_sha256": hashlib.sha256(
            render_playbook(source.read_text(), payload_bytes()).encode()
        ).hexdigest(),
        "binaries": parity["binaries"],
        "versions": {"ruxel": "ruxel 0.1.0", "rustc": "rustc 1.97.1"},
        "fixture": {
            "kind": "disposable-provider",
            "specification": {"image": "debian-12", "server_type": "cpx12"},
        },
        "cases": [
            {
                "case": name,
                "injection_sentinel": True,
                "interrupted_status": 130,
                "reconnect": True,
                "flock_free": True,
                "converged": True,
                "converged_changed": 0,
                "converged_failed": 0,
                "state_equal": True,
                "no_process_leak": True,
                "no_socket_leak": True,
                "no_temp_leak": True,
                "recovery_elapsed_ms": 250,
                "recovery_timeout_ms": 30_000,
            }
            for name in sorted(REQUIRED_CASES)
        ],
    }


class ChaosEvidenceTests(unittest.TestCase):
    def test_complete_manifest_passes(self):
        validate(valid_manifest())

    def test_generated_playbook_hash_is_recomputed(self):
        data = valid_manifest()
        data["generated_playbook_sha256"] = "a" * 64
        with self.assertRaisesRegex(ValueError, "generated playbook hash mismatch"):
            validate(data)

    def test_every_case_is_required_exactly_once(self):
        missing = valid_manifest()
        missing["cases"].pop()
        with self.assertRaisesRegex(ValueError, "matrix mismatch"):
            validate(missing)

        duplicate = valid_manifest()
        duplicate["cases"][-1] = copy.deepcopy(duplicate["cases"][0])
        with self.assertRaisesRegex(ValueError, "duplicated"):
            validate(duplicate)

    def test_each_acceptance_proof_must_be_true(self):
        for field in (
            "injection_sentinel",
            "reconnect",
            "flock_free",
            "converged",
            "state_equal",
            "no_process_leak",
            "no_socket_leak",
            "no_temp_leak",
        ):
            with self.subTest(field=field):
                data = valid_manifest()
                data["cases"][0][field] = False
                with self.assertRaisesRegex(ValueError, field):
                    validate(data)

    def test_interruption_and_convergence_counts_are_strict(self):
        data = valid_manifest()
        data["cases"][0]["interrupted_status"] = 0
        with self.assertRaisesRegex(ValueError, "nonzero"):
            validate(data)

        data = valid_manifest()
        data["cases"][0]["converged_changed"] = 1
        with self.assertRaisesRegex(ValueError, "converged_changed"):
            validate(data)

    def test_recovery_must_be_bounded(self):
        data = valid_manifest()
        data["cases"][0]["recovery_elapsed_ms"] = 30_001
        with self.assertRaisesRegex(ValueError, "exceeded"):
            validate(data)

        data = valid_manifest()
        data["cases"][0]["recovery_timeout_ms"] = 120_001
        with self.assertRaisesRegex(ValueError, "1..120000"):
            validate(data)

    def test_rejects_machine_identity_secrets_paths_and_extra_fields(self):
        for target in (
            "192.0.2.10",
            "2001:db8::1",
            "/Users/operator/run",
            "password=hunter2",
        ):
            with self.subTest(target=target):
                data = valid_manifest()
                data["target"] = target
                with self.assertRaises(ValueError):
                    validate(data)

        data = valid_manifest()
        data["cases"][0]["log"] = "controller output"
        with self.assertRaisesRegex(ValueError, "extra"):
            validate(data)


if __name__ == "__main__":
    unittest.main()
