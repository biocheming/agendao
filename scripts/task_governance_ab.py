#!/usr/bin/env python3
"""task_governance_ab — Phase 6 A/B evaluation harness (stdlib only).

Protocol (docs/plans/j-space-inspired-task-governance-plan.md §Phase 6):

  groups        control | skill | ledger   (combined / pronoun deferred:
                they run only after the first three prove complementary)
  seeds         ≥5 per group; deterministic models record repeats instead
  variables     one — the group. Model, prompt text, tools, workspace
                fixture, permission watcher behavior, and timeouts are
                identical across arms of the same task+seed.
  measured      process completion, independent verification pass,
                unsupported-"done", tokens, cost, wall time, permission
                interventions, tool errors and repairs.
  not measured  recovery/compaction restoration, blank retries, first useful
                action latency, ledger conflicts and stall false positives.
                These require dedicated tasks/event capture and are reported
                as unavailable rather than silently treated as zero.

The harness REPORTS; the plan's default-enablement gates decide. Single
runs never justify enabling anything (plan §1.7).

Usage:
  python3 scripts/task_governance_ab.py \
      --base-url http://127.0.0.1:3987 \
      --binary /path/to/agendao \
      --model deepseek/deepseek-chat \
      --seeds 5 --out /tmp/ab-results.jsonl
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

BASE = ""
LEDGER_SESSION = None  # set per run for ledger-arm goal creation
LEDGER_NEXT_REVISION = 0


def http(method, path, body=None, timeout=15):
    url = f"{BASE}{path}"
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(url, data=data, method=method)
    if data:
        request.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(request, timeout=timeout) as response:
        raw = response.read()
        return json.loads(raw) if raw else None


class PermissionWatcher(threading.Thread):
    """Auto-approves pending permissions for THIS harness's sessions only —
    identically for all arms — never for unrelated sessions on a shared
    server. The allow-set is updated by run_one under the class lock."""

    daemon = True
    stop = threading.Event()
    allowed_sessions = set()
    approvals_by_session = {}
    lock = threading.Lock()

    @classmethod
    def allow(cls, session_id):
        with cls.lock:
            cls.allowed_sessions.add(session_id)
            cls.approvals_by_session.setdefault(session_id, 0)

    @classmethod
    def approval_count(cls, session_id):
        with cls.lock:
            return cls.approvals_by_session.get(session_id, 0)

    def run(self):
        seen = set()
        while not self.stop.is_set():
            try:
                items = http("GET", "/permission", timeout=3)
            except Exception:
                items = []
            with self.lock:
                allowed = set(self.allowed_sessions)
            for item in items or []:
                pid = item.get("id")
                sid = item.get("session_id")
                if not pid or pid in seen:
                    continue
                if sid not in allowed:
                    continue
                try:
                    http("POST", f"/permission/{pid}/reply", {"reply": "turn"}, timeout=5)
                    # Mark seen ONLY after success so a transient failure is
                    # retried on the next poll instead of being dropped.
                    seen.add(pid)
                    with self.lock:
                        self.approvals_by_session[sid] = (
                            self.approvals_by_session.get(sid, 0) + 1
                        )
                except Exception:
                    pass
            time.sleep(1)


TASKS = {
    # Bounded multi-step, verifiable: exercises Core broadcast + done-check.
    "median-tests": {
        "skill_prefix": (
            "Use the j-space skill for this task: first load it via "
            "skills_list/skill_view, follow its gate and modules. "
        ),
        "prompt": (
            "给 stats.py 增加一个 median(values) 函数：空列表必须抛出 "
            'ValueError("empty input")；然后在 tests/test_stats.py 里用纯标准库'
            "编写测试，覆盖 [1,2,3]、[1,2,3,4]（偶数个数取中间两数平均值）和"
            "空列表报错三种情况；运行测试并确认全部通过。"
        ),
        "ledger_goal": "median shipped with 3 passing stdlib tests",
        "ledger_next": "implement median in stats.py",
        "fixture": {
            "README.md": "# Task fixture\n\nA tiny stats library under construction.\n",
            "stats.py": "def mean(values):\n    return sum(values) / len(values)\n",
        },
        "verify": ["python3", "-m", "unittest", "discover", "-s", "tests", "-v"],
    },
}


def reset_fixture(root, task):
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root, exist_ok=True)
    for name, content in task["fixture"].items():
        path = os.path.join(root, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(content)


def create_ledger(session_id, task):
    """Native-ledger arm: establish typed governance before the run."""
    http(
        "PATCH",
        f"/session/{session_id}/task-ledger",
        {
            "expected_revision": 0,
            "op": {
                "op": "create",
                "goal": {
                    "statement": task["ledger_goal"],
                    "acceptance_criteria": ["unittest discover passes 3 cases"],
                    "set_by": "user",
                    "set_at": int(time.time() * 1000),
                },
                "next_statement": task["ledger_next"],
            },
        },
    )


def run_one(binary, model, task_name, task, group, seed, workroot):
    directory = os.path.join(workroot, f"{task_name}-{group}-{seed}")
    reset_fixture(directory, task)

    session = http(
        "POST",
        "/session",
        {"directory": directory, "title": f"ab-{task_name}-{group}-s{seed}"},
    )
    session_id = session["id"]
    PermissionWatcher.allow(session_id)

    if group == "ledger":
        create_ledger(session_id, task)

    message = task["prompt"]
    if group == "skill":
        message = task["skill_prefix"] + task["prompt"]

    started = time.time()
    command = [
        binary,
        "run",
        "--attach",
        BASE,
        "--dir",
        directory,
        "--model",
        model,
        "--title",
        f"ab-{task_name}-{group}-s{seed}",
    ]
    # EVERY arm continues the prepared session: identical isolation
    # conditions across groups, and the permission watcher's allow-set
    # matches the session that actually runs.
    command += ["--session", session_id]
    command.append(message)
    proc = subprocess.run(
        command,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=900,
    )
    elapsed = time.time() - started

    # Independent verification: the harness re-runs the acceptance check
    # itself — a fluent summary in the transcript never counts as done.
    verify_ok = False
    verify_output = ""
    try:
        check = subprocess.run(
            task["verify"],
            cwd=directory,
            capture_output=True,
            text=True,
            timeout=120,
        )
        verify_ok = check.returncode == 0
        verify_output = (check.stdout + check.stderr)[-400:]
    except Exception as error:
        verify_output = f"verify error: {error}"

    # Real binding evidence (NOT a tautological id compare): after the run,
    # the task directory must hold exactly the prepared session, and that
    # session must carry the run's messages.
    dir_sessions = http(
        "GET", f"/session?directory={directory}&limit=10", timeout=15
    ) or {}
    dir_items = (
        dir_sessions.get("items", [])
        if isinstance(dir_sessions, dict)
        else dir_sessions
    )
    messages = http("GET", f"/session/{session_id}/message?limit=5", timeout=15) or []
    message_items = (
        messages if isinstance(messages, list) else messages.get("items", [])
    )
    detail = http("GET", f"/session/{session_id}", timeout=15) or {}
    ledger = http("GET", f"/session/{session_id}/task-ledger", timeout=15) or {}
    telemetry = detail.get("telemetry") or {}
    usage = telemetry.get("usage") or {}
    repair = telemetry.get("tool_repair_summary") or {}
    record = {
        # Binding evidence: exactly one session in the fixture directory
        # (the prepared one) and it carries run messages.
        "sessions_in_fixture_dir": len(dir_items),
        "prepared_session_has_messages": len(message_items) > 0,
        "task": task_name,
        "group": group,
        "seed": seed,
        "session_id": session_id,
        "exit_code": proc.returncode,
        "elapsed_s": round(elapsed, 1),
        "process_completed": proc.returncode == 0,
        "verify_ok": verify_ok,
        "verify_tail": verify_output,
        "ledger_revision": ledger.get("revision", 0),
        "ledger_status": ledger.get("status"),
        "ledger_open": len(
            [q for q in ledger.get("open", []) if not q.get("closed_by_checkpoint_id")]
        ),
        "ledger_verified": len(ledger.get("verified", [])),
        "input_tokens": usage.get("input_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "reasoning_tokens": usage.get("reasoning_tokens"),
        "total_tokens": sum(
            usage.get(name, 0) or 0
            for name in ("input_tokens", "output_tokens", "reasoning_tokens")
        ),
        "provider_cost": usage.get("total_cost"),
        "permission_interventions": PermissionWatcher.approval_count(session_id),
        "total_tool_calls": repair.get("total_tool_calls"),
        "error_tool_calls": repair.get("error_tool_call_count"),
        "repaired_tool_calls": repair.get("repaired_tool_call_count"),
        "session_title": detail.get("title"),
    }
    return record


def summarize(records):
    groups = {}
    for record in records:
        bucket = groups.setdefault(record["group"], [])
        bucket.append(record)
    summary = {}

    def mean_available(items, field, digits=2):
        values = [item[field] for item in items if isinstance(item.get(field), (int, float))]
        return round(sum(values) / len(values), digits) if values else None

    for group, items in sorted(groups.items()):
        n = len(items)
        summary[group] = {
            "n": n,
            "verify_pass_rate": round(sum(1 for r in items if r["verify_ok"]) / n, 3),
            "process_completion_rate": round(
                sum(1 for r in items if r.get("process_completed")) / n, 3
            ),
            "mean_elapsed_s": mean_available(items, "elapsed_s", 1),
            "mean_total_tokens": mean_available(items, "total_tokens", 1),
            "mean_provider_cost": mean_available(items, "provider_cost", 6),
            "mean_permission_interventions": mean_available(
                items, "permission_interventions", 2
            ),
            "mean_total_tool_calls": mean_available(items, "total_tool_calls", 2),
            "mean_error_tool_calls": mean_available(items, "error_tool_calls", 2),
            "mean_repaired_tool_calls": mean_available(items, "repaired_tool_calls", 2),
            "ledger_active": any(r.get("ledger_revision", 0) > 0 for r in items),
            "unsupported_done": sum(
                1
                for r in items
                if r["exit_code"] == 0 and not r["verify_ok"] and r.get("ledger_open", 0) == 0
            ),
            # Isolation contract violations — any non-zero count invalidates
            # the arm's data.
            "binding_violations": sum(
                1
                for r in items
                if r.get("sessions_in_fixture_dir") != 1
                or not r.get("prepared_session_has_messages")
            ),
            "unavailable_metrics": [
                "recovery_or_compaction_state_restoration_rate",
                "blank_retry_count",
                "first_effective_action_latency",
                "ledger_revision_conflict_count",
                "stall_false_positive_rate",
            ],
        }
    return summary


def main():
    global BASE
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--model", default="deepseek/deepseek-chat")
    parser.add_argument("--seeds", type=int, default=5)
    parser.add_argument("--groups", default="control,skill,ledger")
    parser.add_argument("--tasks", default="median-tests")
    parser.add_argument("--out", default="/tmp/task_governance_ab.jsonl")
    args = parser.parse_args()

    BASE = args.base_url.rstrip("/")
    health = http("GET", "/health", timeout=5)
    print(f"server: {health}")

    watcher = PermissionWatcher()
    watcher.start()

    workroot = tempfile.mkdtemp(prefix="task-governance-ab-")
    records = []
    try:
        with open(args.out, "a", encoding="utf-8") as out:
            for task_name in args.tasks.split(","):
                task = TASKS[task_name]
                for group in args.groups.split(","):
                    for seed in range(1, args.seeds + 1):
                        print(f"== {task_name} / {group} / seed {seed}")
                        try:
                            record = run_one(
                                args.binary, args.model, task_name, task, group, seed, workroot
                            )
                        except subprocess.TimeoutExpired:
                            record = {
                                "task": task_name,
                                "group": group,
                                "seed": seed,
                                "exit_code": "timeout",
                                "verify_ok": False,
                            }
                        records.append(record)
                        out.write(json.dumps(record, ensure_ascii=False) + "\n")
                        out.flush()
                        print(f"   verify_ok={record.get('verify_ok')} "
                              f"elapsed={record.get('elapsed_s')}s")
    finally:
        watcher.stop.set()
        shutil.rmtree(workroot, ignore_errors=True)

    summary = summarize(records)
    print("\n== summary (reporting only; plan gates decide) ==")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    with open(args.out.replace(".jsonl", ".summary.json"), "w", encoding="utf-8") as handle:
        json.dump(summary, handle, ensure_ascii=False, indent=2)
    return 0


if __name__ == "__main__":
    sys.exit(main())
