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

    def test_char_literal_does_not_swallow_a_trailing_comment(self):
        self.assertEqual(
            forbidden_comments("let q = '\"'; // tail\n"),
            [(1, "end-of-line comments are forbidden")],
        )

    def test_char_literals_and_lifetimes_pass(self):
        src = "let a = 'x'; let b = '\\n'; fn f<'a>(s: &'a str) -> &'a str { s }\n"
        self.assertEqual(forbidden_comments(src), [])

    def test_doc_of_fifteen_lines_passes(self):
        self.assertEqual(long_doc_comments("/// x\n" * 15 + "fn a() {}\n"), [])

    def test_doc_of_sixteen_lines_fails(self):
        self.assertEqual(long_doc_comments("/// x\n" * 16 + "fn a() {}\n"), [(1, "doc comment of 16 lines (limit 15)")])

    def test_doc_run_split_by_attribute_still_counts(self):
        src = "/// x\n" * 10 + "#[cfg(unix)]\n" + "/// x\n" * 10 + "fn a() {}\n"
        self.assertEqual(long_doc_comments(src), [(1, "doc comment of 20 lines (limit 15)")])

    def test_doc_run_across_blank_lines_counts(self):
        src = "/// x\n" * 10 + "\n" + "/// x\n" * 10 + "fn a() {}\n"
        self.assertEqual(long_doc_comments(src), [(1, "doc comment of 20 lines (limit 15)")])


class TestRules(unittest.TestCase):
    def test_asserting_test_passes(self):
        self.assertEqual(test_rules("#[test]\nfn a() { assert_eq!(1, 1); }\n"), [])

    def test_empty_test_fails(self):
        self.assertEqual(test_rules("#[test]\nfn a() { let _ = 1; }\n"), [(1, "test has no assertion")])

    def test_should_panic_needs_no_assertion(self):
        self.assertEqual(test_rules("#[test]\n#[should_panic]\nfn a() { boom(); }\n"), [])

    def test_sleeping_test_fails(self):
        findings = test_rules("#[test]\nfn a() { thread::sleep(d); assert!(x); }\n")
        self.assertEqual(findings, [(1, "test sleeps; poll with a timeout instead")])

    def test_result_style_test_counts_as_asserting(self):
        body = '#[test]\nfn a() -> Result<(), E> { match f() { Ok(_) => return Err(fail("no")), _ => {} } Ok(()) }\n'
        self.assertEqual(test_rules(body), [])

    def test_sleep_inside_a_string_is_not_a_sleeping_test(self):
        body = '#[test]\nfn a() { cmd.args(["-c", "sleep 30"]); assert!(x); }\n'
        self.assertEqual(test_rules(body), [])

    def test_assertion_inside_a_string_does_not_count(self):
        body = '#[test]\nfn a() { let s = "assert!(x)"; }\n'
        self.assertEqual(test_rules(body), [(1, "test has no assertion")])

    def test_sleep_call_inside_a_string_does_not_count(self):
        body = '#[test]\nfn a() { let s = "thread::sleep(d)"; assert!(x); }\n'
        self.assertEqual(test_rules(body), [])

    def test_brace_inside_char_literal_does_not_hide_the_body(self):
        body = '#[test]\nfn a() { let c = \'{\'; assert!(x); }\n'
        self.assertEqual(test_rules(body), [])

    def test_bare_ignore_fails(self):
        self.assertEqual(len(test_rules("#[ignore]\n#[test]\nfn a() { assert!(x); }\n")), 1)

    def test_ignore_with_reason_passes(self):
        self.assertEqual(test_rules('#[ignore = "needs a device"]\n#[test]\nfn a() { assert!(x); }\n'), [])


if __name__ == "__main__":
    unittest.main()
