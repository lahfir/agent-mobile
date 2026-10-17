"""Device-free tests for the agent-task harness."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

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

    def test_relative_binary_becomes_absolute_because_claude_runs_in_another_directory(self):
        servers = agent.server_map(None, "target/release/agent-mobile")
        self.assertTrue(pathlib.Path(servers["agent-mobile"]["command"]).is_absolute())

    def test_builtin_server_passes_the_device_to_the_binary(self):
        servers = agent.server_map(None, "/tmp/agent-mobile", "android:avd:x")
        self.assertEqual(servers["agent-mobile"]["args"], ["--device", "android:avd:x", "mcp"])

    def test_malformed_file_fails_before_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "servers.json"
            path.write_text("{")
            with self.assertRaises(json.JSONDecodeError):
                agent.server_map(path)

    def test_disallowed_tools_must_be_a_list_of_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "servers.json"
            path.write_text(json.dumps({"server-a": {"command": "stub", "disallowedTools": "x"}}))
            with self.assertRaises(ValueError):
                agent.server_map(path)

    def test_teardown_must_be_a_list_of_command_lists(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "servers.json"
            path.write_text(json.dumps({"server-a": {"command": "stub", "teardown": ["adb"]}}))
            with self.assertRaises(ValueError):
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
        original_serial = agent.android_serial
        agent.android_sdk = lambda: ("/sdk/adb", "/sdk/emulator")
        agent.android_serial = lambda adb, avd: "emulator-5554"
        try:
            for task in tasks.TASKS:
                for seed in range(len(task["values"])):
                    _, values = tasks.task_variant(task, seed)
                    for platform, check in task["checks"].items():
                        device = "android:avd:a" if platform == "android" else "ios:UDID"
                        for command in [*task["reset"][platform], check["command"]]:
                            self.assertTrue(agent.fill(command, values, device))
                        self.assertTrue(check["expected"].format(**values))
        finally:
            agent.android_sdk = original
            agent.android_serial = original_serial

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
        original, original_serial = agent.android_sdk, agent.android_serial
        agent.android_sdk = lambda: ("/sdk/adb", "/sdk/emulator")
        agent.android_serial = lambda adb, avd: "emulator-5554"
        try:
            command = agent.fill(["adb", "shell", "settings get {value}"], {"value": "x"}, "android:avd:a")
        finally:
            agent.android_sdk, agent.android_serial = original, original_serial
        self.assertEqual(command, ["/sdk/adb", "-s", "emulator-5554", "shell", "settings get x"])

    def test_fill_passes_a_serial_key_straight_to_adb(self):
        with mock.patch.object(agent, "android_sdk", return_value=("/sdk/adb", "/sdk/emulator")):
            command = agent.fill(["adb", "shell", "id"], {}, "android:emulator-5556")
        self.assertEqual(command, ["/sdk/adb", "-s", "emulator-5556", "shell", "id"])

    def test_timeout_with_a_missing_server_is_invalid(self):
        init = json.dumps({"type": "system", "subtype": "init", "mcp_servers": []})
        class Process:
            pid = 123
            returncode = -9
            def communicate(self, timeout=None):
                if timeout is not None:
                    raise subprocess.TimeoutExpired("claude", timeout)
                return init + "\n", ""
        with mock.patch.object(agent.subprocess, "Popen", return_value=Process()), \
             mock.patch.object(agent.os, "killpg"):
            result = agent.run_claude("goal", "server-a", {"command": "stub"}, "/tmp", timeout=1)
        self.assertEqual(result["status"], "invalid")

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

    def test_check_command_error_is_an_invalid_attempt(self):
        failed = agent.checked_outcome({"status": "agent_finished"}, {"command": ["missing"], "expected": "yes"}, {},
                                       run=lambda *a, **k: (_ for _ in ()).throw(FileNotFoundError("missing")))
        self.assertEqual((failed["status"], failed["cause"]), ("invalid", "missing"))

    def test_photo_check_propagates_query_failure_before_counting(self):
        command = tasks.TASKS[7]["checks"]["android"]["command"][2]
        self.assertIn("|| exit $?", command)
        self.assertIn("awk '/Row/", command)
        self.assertNotIn("|| true", command)

    def test_timeout_kills_the_claude_process_group(self):
        class Process:
            pid = 123
            returncode = -9
            def communicate(self, timeout=None):
                if timeout is not None:
                    raise subprocess.TimeoutExpired("claude", timeout)
                return "", ""
        with mock.patch.object(agent.subprocess, "Popen", return_value=Process()) as popen, \
             mock.patch.object(agent.os, "killpg") as killpg:
            result = agent.run_claude("goal", "server-a", {"command": "stub"}, "/tmp", timeout=1)
        self.assertEqual(result["reason"], "timeout")
        self.assertTrue(popen.call_args.kwargs["start_new_session"])
        killpg.assert_called_once_with(123, agent.signal.SIGKILL)

    def test_disallowed_tools_leave_the_agent_view_and_the_config(self):
        class Process:
            pid = 123
            returncode = 0
            def communicate(self, timeout=None):
                return "", ""
        seen = {}

        def popen(command, **kwargs):
            seen["command"] = command
            seen["config"] = json.loads(pathlib.Path(command[command.index("--mcp-config") + 1]).read_text())
            return Process()

        with mock.patch.object(agent.subprocess, "Popen", side_effect=popen):
            agent.run_claude("goal", "server-a", {"command": "stub", "disallowedTools": ["a", "b"]}, "/tmp")
        index = seen["command"].index("--disallowedTools")
        self.assertEqual(seen["command"][index + 1], "mcp__server-a__a,mcp__server-a__b")
        self.assertEqual(seen["config"], {"mcpServers": {"server-a": {"command": "stub"}}})

    def test_teardown_stays_out_of_the_config(self):
        class Process:
            pid = 123
            returncode = 0
            def communicate(self, timeout=None):
                return "", ""
        seen = {}

        def popen(command, **kwargs):
            seen["config"] = json.loads(pathlib.Path(command[command.index("--mcp-config") + 1]).read_text())
            return Process()

        with mock.patch.object(agent.subprocess, "Popen", side_effect=popen):
            agent.run_claude("goal", "server-a", {"command": "stub", "teardown": [["adb", "shell", "id"]]}, "/tmp")
        self.assertEqual(seen["config"], {"mcpServers": {"server-a": {"command": "stub"}}})

    def test_teardown_runs_every_servers_commands_and_swallows_os_errors(self):
        servers = {"agent-mobile": {"command": "x"},
                   "server-a": {"command": "stub", "teardown": [["one"], ["two", "{device}"]]},
                   "server-b": {"command": "stub", "teardown": [["three"]]}}
        seen = []
        agent.teardown_all(servers, "ios:U", run=lambda c, **k: seen.append((c, k["timeout"], k["check"])))
        self.assertEqual(seen, [(["one"], 30, False), (["two", "U"], 30, False), (["three"], 30, False)])

        def missing(*a, **k):
            raise OSError("adb not found")
        self.assertIsNone(agent.teardown_all(servers, "ios:U", run=missing))

    def test_stdin_is_devnull_and_keep_writes_stream_and_stderr(self):
        class Process:
            pid = 123
            returncode = 0
            def communicate(self, timeout=None):
                return "out-line\n", "err-line\n"
        with tempfile.TemporaryDirectory() as tmp, \
             mock.patch.object(agent.subprocess, "Popen", return_value=Process()) as popen:
            keep = pathlib.Path(tmp) / "deep" / "run-a1"
            agent.run_claude("goal", "server-a", {"command": "stub"}, "/tmp", keep=keep)
            self.assertEqual((keep.parent / "run-a1.jsonl").read_text(), "out-line\n")
            self.assertEqual((keep.parent / "run-a1.err").read_text(), "err-line\n")
        self.assertIs(popen.call_args.kwargs["stdin"], subprocess.DEVNULL)

    def test_warmup_runs_once_per_server_per_platform_and_lands_in_meta(self):
        goals = []
        servers = {"agent-mobile": {"command": "x"}, "server-a": {"command": "y"}}
        done = {"status": "success", "wall_ms": 1, "steps": 0, "stale_refs": None}
        with tempfile.TemporaryDirectory() as tmp, \
             mock.patch.object(agent, "OUT", pathlib.Path(tmp)), \
             mock.patch.object(agent, "server_map", return_value=servers), \
             mock.patch.object(agent, "pick_device", return_value={"key": "android:avd:a"}), \
             mock.patch.object(agent.subprocess, "run", return_value=mock.Mock(stdout="v1")), \
             mock.patch.object(agent, "run_claude", side_effect=lambda goal, *a, **k: goals.append((goal, a[0], k["keep"].name))), \
             mock.patch.object(agent, "run_one", return_value=done), \
             mock.patch("scripts.benchkit.report.render_agent"):
            result = agent.run_agent(None, tasks="dark-mode", seeds=2, platforms="android")
        self.assertEqual(goals, [(agent.WARMUP_GOAL, "agent-mobile", "android-agent-mobile-warmup"),
                                 (agent.WARMUP_GOAL, "server-a", "android-server-a-warmup")])
        self.assertEqual(set(result["meta"]["warmup_ms"]), {"agent-mobile:android", "server-a:android"})

    def test_init_status_failed_or_missing_is_a_fault_and_other_states_are_not(self):
        def init(*servers):
            return [json.dumps({"type": "system", "subtype": "init", "mcp_servers": list(servers)})]
        for status, failed in (("connected", []), ("failed", ["server-a"]), ("pending", []),
                               ("needs-auth", []), ("disabled", [])):
            parsed = agent.parse_stream(init({"name": "server-a", "status": status}), "server-a")
            self.assertEqual(parsed["failed"], failed, status)
        self.assertEqual(agent.parse_stream(init(), "server-a")["failed"], ["server-a"])

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

    def test_teardown_runs_before_each_reset(self):
        calls = []
        with mock.patch.object(agent, "teardown_all", side_effect=lambda *a, **k: calls.append("teardown")), \
             mock.patch.object(agent, "reset_device", side_effect=lambda *a, **k: calls.append("reset") or "adb offline"):
            row = agent.run_one("agent-mobile", {}, tasks.TASKS[0], "android", 0, "android:avd:a", servers={})
        self.assertEqual(calls, ["teardown", "reset", "teardown", "reset"])
        self.assertEqual(row["attempts"], 2)

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

    def test_invalid_retry_accounting_survives_success(self):
        outcomes = iter([{"status": "invalid", "cause": "first check error"}, {"status": "success"}])
        original = agent.reset_device, agent.run_claude, agent.checked_outcome
        agent.reset_device = lambda *a, **k: None
        agent.run_claude = lambda *a, **k: next(outcomes)
        agent.checked_outcome = lambda outcome, *a, **k: outcome
        try:
            row = agent.run_one("server-a", {}, tasks.TASKS[0], "android", 0, "android:avd:a")
        finally:
            agent.reset_device, agent.run_claude, agent.checked_outcome = original
        self.assertEqual((row["status"], row["invalid_attempts"], row["invalid_causes"]),
                         ("success", 1, ["first check error"]))

    def test_agent_report_slide_helper_is_shared_and_script_data_is_escaped(self):
        html = (pathlib.Path(__file__).parent / "bench.html").read_text()
        self.assertLess(html.index("const slide ="), html.index("function agentReport()"))
        from scripts.benchkit import report
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "report.html"
            report.render_agent({"agent_tasks": [{"cause": "</script><script>alert(1)</script>"}]}, path)
            rendered = path.read_text()
        self.assertNotIn("</script><script>alert(1)", rendered)
        self.assertIn("\\u003c/script>", rendered)
        self.assertIn("${esc(x.cause || x.reason || \"\")}", html)

    def test_server_that_fails_to_connect_is_a_fault_not_a_failed_task(self):
        init = json.dumps({"type": "system", "subtype": "init", "mcp_servers": [{"name": "s", "status": "failed"}]})
        self.assertEqual(agent.parse_stream([init], "s")["failed"], ["s"])
        empty = json.dumps({"type": "system", "subtype": "init", "mcp_servers": []})
        self.assertEqual(agent.parse_stream([empty], "s")["failed"], ["s"])


if __name__ == "__main__":
    unittest.main()
