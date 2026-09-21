#!/usr/bin/env python3
import unittest

from check_rust_comments import forbidden_comments, long_doc_comments, test_rules


class CommentRules(unittest.TestCase):
    def test_doc_comments_pass(self):
        self.assertEqual(forbidden_comments("/// doc\n//! inner\nfn a() {}\n"), [])

    def test_inline_comment_fails(self):
        self.assertEqual(forbidden_comments("fn a() {} // why\n"), [(1, "end-of-line comments are forbidden")])

    def test_line_comment_fails(self):
        self.assertEqual(forbidden_comments("// note\nfn a() {}\n"), [(1, "non-doc line comments are forbidden")])

    def test_slashes_inside_strings_pass(self):
        self.assertEqual(forbidden_comments('let u = "http://x/y"; let r = r#"//"#;\n'), [])

    def test_doc_of_fifteen_lines_passes(self):
        self.assertEqual(long_doc_comments("/// x\n" * 15 + "fn a() {}\n"), [])

    def test_doc_of_sixteen_lines_fails(self):
        self.assertEqual(long_doc_comments("/// x\n" * 16 + "fn a() {}\n"), [(1, "doc comment of 16 lines (limit 15)")])


class TestRules(unittest.TestCase):
    def test_asserting_test_passes(self):
        self.assertEqual(test_rules("#[test]\nfn a() { assert_eq!(1, 1); }\n"), [])

    def test_empty_test_fails(self):
        self.assertEqual(test_rules("#[test]\nfn a() { let _ = 1; }\n"), [(1, "test has no assertion")])

    def test_should_panic_needs_no_assertion(self):
        self.assertEqual(test_rules("#[test]\n#[should_panic]\nfn a() { boom(); }\n"), [])

    def test_sleeping_test_fails(self):
        findings = test_rules("#[test]\nfn a() { thread::sleep(d); assert!(x); }\n")
        self.assertEqual(findings, [(1, "test sleeps; make it deterministic")])

    def test_bare_ignore_fails(self):
        self.assertEqual(len(test_rules("#[ignore]\n#[test]\nfn a() { assert!(x); }\n")), 1)

    def test_ignore_with_reason_passes(self):
        self.assertEqual(test_rules('#[ignore = "needs a device"]\n#[test]\nfn a() { assert!(x); }\n'), [])


if __name__ == "__main__":
    unittest.main()
