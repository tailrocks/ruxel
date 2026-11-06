import unittest

from coverage import admission_kinds, controller_kinds, dispatch_kinds, operation_kinds


class CoverageTest(unittest.TestCase):
    def test_extracts_only_operation_classes_and_rejects_duplicates(self):
        source = '''typealias Operation = Copy|File
class Value { fixed kind = "value" }
class Copy { fixed kind = "copy"; path: String }
class File { fixed kind = "file" }'''
        self.assertEqual(operation_kinds(source), {"copy", "file"})
        with self.assertRaises(ValueError):
            operation_kinds(source.replace('fixed kind = "file"', 'fixed kind = "copy"'))
        with self.assertRaises(ValueError):
            operation_kinds(source.replace('Copy|File', 'Copy|Missing'))

    def test_controller_dispatch(self):
        operation = 'if matches!(kind, "assert" | "debug") { return self.local_retry('
        local = 'async fn local_once() { match kind {\n "assert" => {},\n "debug" => {},\n _ => {}\n } } async fn local_retry() { self.local_once('
        self.assertEqual(controller_kinds(operation, local), {"assert", "debug"})
        for bad_operation, bad_local in [
            (operation.replace(' | "debug"', ''), local),
            (operation.replace('local_retry', 'other'), local),
            (operation + operation, local),
            (operation, local.replace('"debug" =>', '"other" =>')),
            (operation, local.replace('self.local_once(', 'self.other(')),
            (operation, local.replace('async fn local_once', 'async fn other')),
        ]:
            with self.assertRaises(ValueError):
                controller_kinds(bad_operation, bad_local)

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
