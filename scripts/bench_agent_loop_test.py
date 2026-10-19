"""bench_agent_loop: a protocol error must fail the run, never pass it."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("bench", pathlib.Path(__file__).with_name("bench_agent_loop.py"))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class ErrorServer(bench.Mcp):
    def __init__(self):
        pass

    def rpc(self, method, params):
        return {"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "Unknown tool"}}


class ProtocolErrorTest(unittest.TestCase):
    def test_error_envelope_fails_the_platform(self):
        out = bench.run_platform(ErrorServer(), "ios", smoke=True)
        self.assertFalse(out["ok"])
        self.assertIn("protocol error", out["fail"])


if __name__ == "__main__":
    unittest.main()
