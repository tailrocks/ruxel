import unittest

from coverage import admission_kinds, dispatch_kinds, operation_kinds


class CoverageTest(unittest.TestCase):
    def test_extracts_only_operation_classes_and_rejects_duplicates(self):
        source = '''class Value extends Expr { kind = "value" }
class Copy extends Operation { kind = "copy"; path: String }
class File extends Operation { kind = "file" }'''
        self.assertEqual(operation_kinds(source), {"copy", "file"})
        with self.assertRaises(ValueError):
            operation_kinds(source + '\nclass Other extends Operation { kind = "copy" }')

    def test_dispatch_admission_is_extracted_and_missing_guard_is_allowed(self):
        source = 'fn is_implemented(module: &str) -> bool { matches!(module, "copy" | "file") }'
        self.assertEqual(admission_kinds(source), {"copy", "file"})
        self.assertIsNone(admission_kinds("fn execute() {}"))
        with self.assertRaises(ValueError):
            admission_kinds("fn is_implemented(x: &str) -> bool { custom(x) }")

    def test_uses_dispatch_arms_not_supported_list(self):
        source = '''let result = match module {
    "copy" | "template" => copy::run(params),
    "file" => file::run(params),
    _ => unreachable!(),
};
fn is_implemented() { "missing"; }'''
        self.assertEqual(dispatch_kinds(source), {"copy", "template", "file"})
        with self.assertRaises(ValueError):
            dispatch_kinds('fn is_implemented() { "copy"; }')


if __name__ == "__main__":
    unittest.main()
