import json
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]


class SchemaTests(unittest.TestCase):
    def evaluate(self, source):
        with tempfile.NamedTemporaryFile(mode="w", suffix=".pkl", dir=ROOT / "tests/schema") as file:
            file.write(source)
            file.flush()
            return subprocess.run(["pkl", "eval", file.name, "-f", "json"], capture_output=True, text=True)

    def test_runtime_and_operations_render(self):
        output = subprocess.check_output(["pkl", "eval", "tests/schema/valid.pkl", "-f", "json"], cwd=ROOT, text=True)
        config = json.loads(output)["config"]
        tasks = config["plays"][0]["tasks"]
        self.assertEqual(tasks[0]["operation"]["owner"], {"kind": "ref", "path": ["vars", "owner"]})
        self.assertEqual(config["plays"][0]["preTasks"][0]["operation"]["argv"], ["true"])
        self.assertEqual(tasks[0]["loopLabel"]["value"], "database")
        self.assertEqual(tasks[2]["block"][0]["operation"]["mode"], "0644")
        self.assertEqual(tasks[1]["operation"]["kind"], "community.general.lvol")
        self.assertEqual(tasks[3]["operation"]["password"]["provider"], "synthetic")

    def test_rejects_invalid_types_and_missing_required_fields(self):
        prefix = 'import "../../schema/Ruxel.pkl" as R\n'
        for expression in [
            'new R.Command {}',
            'new R.Command { cmd = "x"; argv = new Listing<String> { "x" } }',
            'new R.Service { state = "unsupported" }',
            'new R.User { name = "test"; uid = -1 }',
            'new R.File { mode = "not-octal" }',
            'new R.Task { name = "bad"; operation = new R.Debug {}; block { new R.Task { name = "inner"; operation = new R.Debug {} } } }',
            'new R.Operation { kind = "unsupported" }',
            'new R.Apt { kind = "unsupported"; name = "hello" }',
            'new R.Expr { kind = "unsupported" }',
            'new R.Call { `function` = "equals" }',
            'new R.Task { name = "bad"; operation = new { kind = "unsupported" } }',
            'new R.Filesystem { dev = "/dev/test"; fstype = "btrfs" }',
            'new R.LogicalVolume { vg = "test"; lv = "data" }',
            'new R.Task { name = "bad"; retries = 0 }',
            'new R.Host { address = "localhost"; port = 70000 }',
        ]:
            with self.subTest(expression=expression):
                self.assertNotEqual(self.evaluate(prefix + 'value = ' + expression).returncode, 0)


if __name__ == "__main__":
    unittest.main()
