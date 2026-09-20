#!/usr/bin/env python3
import unittest

from check_rust_comments import forbidden_comments, long_doc_comments


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


if __name__ == "__main__":
    unittest.main()
