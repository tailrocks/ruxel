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
        self.assertEqual(config["plays"][0]["preTasks"][1]["operation"]["argv"], ["true"])
        self.assertEqual(config["plays"][0]["preTasks"][2]["operation"]["seconds"], 1)
        self.assertEqual(tasks[0]["loopLabel"]["value"], "database")
        self.assertEqual(tasks[5]["operation"]["state"]["value"], "absent")
        self.assertEqual(tasks[6]["operation"]["type"]["value"], "default_privs")
        self.assertTrue(tasks[9]["operation"]["that"])
        self.assertEqual(tasks[10]["operation"]["content"]["function"], "string")
        self.assertEqual(tasks[11]["operation"]["content"], "data")
        self.assertEqual(tasks[2]["block"][0]["operation"]["mode"], "0644")
        self.assertEqual(tasks[1]["operation"]["kind"], "community.general.lvol")
        self.assertEqual(tasks[12]["operation"]["password"]["provider"], "synthetic")

    def test_rejects_invalid_types_and_missing_required_fields(self):
        prefix = 'import "../../schema/Ruxel.pkl" as R\n'
        for expression in [
            'new R.Command {}',
            'new R.Timezone { name = "UTC"; state = "present" }',
            'new R.PostgresDatabase { name = "db"; login_db = "ignored" }',
            'new R.PostgresRole { name = "role"; login_db = "ignored" }',
            'new R.Command { cmd = "true"; removes = "/tmp/a" }',
            'new R.Shell { cmd = "true"; removes = "/tmp/a" }',
            'new R.Shell {}',
            'new R.Shell { argv = new Listing<String> { "echo" } }',
            'new R.BlockInFile { path = "/tmp/x" }',
            'new R.LineInFile { path = "/tmp/x" }',
            'new R.LineInFile { path = "/tmp/x"; state = "absent" }',
            'new R.PostgresSchema { name = "a" }',
            'new R.PostgresPrivileges { role = "a"; type = "table" }',
            'new R.PostgresPrivileges { login_db = "db"; role = "a"; type = "database" }',
            'new R.PostgresPrivileges { login_db = "db"; role = "a"; type = "table"; target_roles = "a" }',
            'new R.VolumeGroup { vg = "a" }',
            'new R.Sysctl { name = "a" }',
            'new R.Service { state = "started" }',
            'new R.Service { enabled = true }',
            'new R.Firewall { chain = "INPUT"; policy = "DROP"; jump = "ACCEPT" }',
            'new R.Lookup { provider = "pipe" }',
            'new R.Command { argv = "echo hello" }',
            'new R.Apt { upgrade = "garbage" }',
            'new R.File { path = "/tmp/link"; state = "link" }',
            'new R.File { path = "a"; dest = "b" }',
            'new R.Mount { path = "/tmp/mount" }',
            'new R.Template { src = "a"; dest = "b"; content = "c" }',
            'new R.Assert {}',
            'new R.Lookup { provider = "command" }',
            'new R.Command { argv = new Listing<String> {} }',
            'new R.Ref {}',
            'new R.Call { `function` = "eq" }',
            'new R.Call { `function` = "if"; args { new R.Literal { value = true } } }',
            'new R.Call { `function` = "map"; args { new R.Literal { value = true } } }',
            'new R.Apt { state = "garbage" }',
            'new R.File { path = "/tmp/file"; state = "garbage" }',
            'new R.File { mode = "0644" }',
            'new R.Copy { dest = "/tmp/file" }',
            'new R.Copy { dest = "/tmp/file"; src = "a"; content = "b" }',
            'new R.PostgresRole { name = "a"; login_port = 70000 }',
            'new R.PostgresPrivileges { role = "a"; type = "garbage" }',
            'new R.Pause { seconds = -1 }',
            'new R.Command { cmd = "x"; argv = new Listing<String> { "x" } }',
            'new R.Service { state = "unsupported" }',
            'new R.User { name = "test"; uid = -1 }',
            'new R.File { path = "/tmp/file"; mode = "not-octal" }',
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
