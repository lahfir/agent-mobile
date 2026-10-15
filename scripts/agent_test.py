"""Device-free tests for the agent-task harness."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
from scripts.benchkit import agent, tasks  # noqa: E402


class StreamTests(unittest.TestCase):
    def test_counts_tools_stale_refs_and_final_result(self):
        fixture = [
            json.dumps({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "snapshot"}]}}),
            json.dumps({"type": "user", "message": {"content": [{"type": "tool_result", "content": "STALE_REF"}]}}),
            json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": "done"}),
        ]
        parsed = agent.parse_stream(fixture, "agent-mobile")
        self.assertEqual(parsed["steps"], 1)
        self.assertEqual(parsed["stale_refs"], 1)
        self.assertEqual(parsed["final"]["result"], "done")

    def test_other_server_stale_refs_are_not_applicable(self):
        parsed = agent.parse_stream(["{}"], "server-a")
        self.assertIsNone(parsed["stale_refs"])


class ServerMapTests(unittest.TestCase):
    def test_missing_file_uses_only_builtin(self):
        servers = agent.server_map(None, "/tmp/agent-mobile")
        self.assertEqual(servers["agent-mobile"], {"command": "/tmp/agent-mobile", "args": ["mcp"], "env": {}})

    def test_builtin_server_passes_the_device_to_the_binary(self):
        servers = agent.server_map(None, "/tmp/agent-mobile", "android:avd:x")
        self.assertEqual(servers["agent-mobile"]["args"], ["--device", "android:avd:x", "mcp"])

    def test_malformed_file_fails_before_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "servers.json"
            path.write_text("{")
            with self.assertRaises(json.JSONDecodeError):
                agent.server_map(path)

    def test_failed_host_check_keeps_evidence(self):
        class Result:
            stdout = "30000\n"
            stderr = ""

        ok, evidence = agent.host_check({"command": ["fixture"], "expected": "{value}"}, {"value": "60000"},
                                        run=lambda *a, **k: Result())
        self.assertFalse(ok)
        self.assertEqual(evidence, "30000")

    def test_stub_answers_handshake_list_and_call(self):
        command = [sys.executable, "-m", "scripts.benchkit.stub_server"]
        requests = [{"jsonrpc": "2.0", "id": i, "method": method,
                     "params": {"protocolVersion": "2025-11-25"} if method == "initialize" else {}}
                    for i, method in enumerate(("initialize", "tools/list", "tools/call"), 1)]
        process = subprocess.run(command, input="\n".join(map(json.dumps, requests)) + "\n",
                                 capture_output=True, text=True, check=True)
        replies = [json.loads(line) for line in process.stdout.splitlines()]
        self.assertEqual(len(replies), 3)
        self.assertEqual(replies[0]["result"]["protocolVersion"], "2025-11-25")
        self.assertEqual(replies[1]["result"]["tools"][0]["name"], "ping")
        self.assertEqual(replies[2]["result"]["content"][0]["text"], "pong")


class TaskDataTests(unittest.TestCase):
    def test_ten_tasks_have_seed_variants_and_a_reset_for_every_check(self):
        self.assertEqual(len(tasks.TASKS), 10)
        for task in tasks.TASKS:
            self.assertTrue(task["values"])
            self.assertEqual(set(task["checks"]), set(task["reset"]))
            self.assertLessEqual(set(task["checks"]), {"ios", "android"})

    def test_every_task_has_an_android_check_and_six_checks_in_total(self):
        self.assertTrue(all("android" in task["checks"] for task in tasks.TASKS))
        self.assertGreaterEqual(sum(len(task["checks"]) for task in tasks.TASKS), 6)

    def test_seed_variant_formats_goal(self):
        goal, values = tasks.task_variant(tasks.TASKS[0], 1)
        self.assertIn(values["value"], goal)

    def test_every_command_formats_for_every_seed(self):
        original = agent.android_sdk
        agent.android_sdk = lambda: ("/sdk/adb", "/sdk/emulator")
        try:
            for task in tasks.TASKS:
                for seed in range(len(task["values"])):
                    _, values = tasks.task_variant(task, seed)
                    for platform, check in task["checks"].items():
                        for command in [*task["reset"][platform], check["command"]]:
                            self.assertTrue(agent.fill(command, values, "ios:UDID"))
                        self.assertTrue(check["expected"].format(**values))
        finally:
            agent.android_sdk = original

    def test_dark_mode_resets_to_the_opposite_of_its_target(self):
        for seed in (0, 1):
            _, values = tasks.task_variant(tasks.TASKS[0], seed)
            self.assertNotEqual(values["from"], values["value"])
            self.assertNotEqual(values["from_night"], values["night"])


class HarnessTests(unittest.TestCase):
    def test_fill_builds_reset_command_with_udid_and_values(self):
        command = agent.fill(tasks.TASKS[0]["reset"]["ios"][0], {"from": "light"}, "ios:ABC-123")
        self.assertEqual(command, ["xcrun", "simctl", "ui", "ABC-123", "appearance", "light"])

    def test_fill_resolves_adb_to_the_sdk_binary(self):
        original = agent.android_sdk
        agent.android_sdk = lambda: ("/sdk/adb", "/sdk/emulator")
        try:
            command = agent.fill(["adb", "shell", "settings get {value}"], {"value": "x"}, "android:avd:a")
        finally:
            agent.android_sdk = original
        self.assertEqual(command, ["/sdk/adb", "shell", "settings get x"])

    def test_reset_device_runs_each_command_and_reports_failure(self):
        seen = []
        task = {"reset": {"ios": [["one", "{a}"], ["two"]]}}
        self.assertIsNone(agent.reset_device(task, "ios", {"a": "1"}, "ios:U", run=lambda c, **k: seen.append(c)))
        self.assertEqual(seen, [["one", "1"], ["two"]])

        def boom(*a, **k):
            raise subprocess.CalledProcessError(1, "two")
        self.assertIn("returned non-zero", agent.reset_device(task, "ios", {"a": "1"}, "ios:U", run=boom))

    def test_check_failure_records_evidence_and_timeout_stays_unchecked(self):
        check = {"command": ["fixture"], "expected": "yes"}

        class Result:
            stdout = "no\n"
            stderr = ""

        run = lambda *a, **k: Result()  # noqa: E731
        failed = agent.checked_outcome({"status": "agent_finished"}, check, {}, run=run)
        self.assertEqual((failed["status"], failed["reason"]), ("success_false", "no"))
        timeout = {"status": "success_false", "reason": "timeout"}
        self.assertEqual(agent.checked_outcome(timeout, check, {}, run=lambda *a, **k: Result()), timeout)
        Result.stdout = "yes\n"
        self.assertEqual(agent.checked_outcome({"status": "agent_finished"}, check, {}, run=run)["status"], "success")

    def test_plan_filters_tasks_seeds_and_platforms(self):
        plan = agent.run_plan("dark-mode,alarm", 2, "android")
        self.assertEqual([(p, t["id"], s) for p, t, s in plan],
                         [("android", "dark-mode", 0), ("android", "dark-mode", 1),
                          ("android", "alarm", 0), ("android", "alarm", 1)])
        ios = agent.run_plan(None, 1, "ios")
        self.assertEqual([t["id"] for _, t, _ in ios], [t["id"] for t in tasks.TASKS if "ios" in t["checks"]])

    def test_plan_rejects_unknown_names_before_any_device_work(self):
        with self.assertRaises(SystemExit):
            agent.run_plan("nope", 1, None)
        with self.assertRaises(SystemExit):
            agent.run_plan(None, 1, "watch")

    def test_harness_fault_reruns_once_and_stays_invalid(self):
        original = agent.reset_device, agent.run_claude
        agent.reset_device = lambda *a, **k: "adb offline"
        agent.run_claude = lambda *a, **k: self.fail("no agent run after a failed reset")
        try:
            row = agent.run_one("agent-mobile", {}, tasks.TASKS[0], "android", 0, "android:avd:a")
        finally:
            agent.reset_device, agent.run_claude = original
        self.assertEqual((row["status"], row["attempts"]), ("invalid", 2))
        self.assertIn("adb offline", row["cause"])

    def test_invalid_then_valid_run_keeps_the_second_attempt(self):
        outcomes = iter([{"status": "invalid", "cause": "claude CLI error"},
                         {"status": "success_false", "reason": "timeout", "wall_ms": 1}])
        original = agent.reset_device, agent.run_claude
        agent.reset_device = lambda *a, **k: None
        agent.run_claude = lambda *a, **k: next(outcomes)
        try:
            row = agent.run_one("agent-mobile", {}, tasks.TASKS[0], "android", 0, "android:avd:a")
        finally:
            agent.reset_device, agent.run_claude = original
        self.assertEqual((row["status"], row["reason"], row["attempts"]), ("success_false", "timeout", 2))

    def test_server_that_fails_to_connect_is_a_fault_not_a_failed_task(self):
        init = json.dumps({"type": "system", "subtype": "init", "mcp_servers": [{"name": "s", "status": "failed"}]})
        self.assertEqual(agent.parse_stream([init], "s")["failed"], ["s"])


if __name__ == "__main__":
    unittest.main()
