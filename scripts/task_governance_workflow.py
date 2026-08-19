"""Workflow execution and recovery probes for the governance harness."""

import os
import subprocess
import time
import urllib.parse


def process_result(exit_code, stdout, stderr, started):
    return {
        "exit_code": exit_code,
        "stdout_tail": stdout[-1000:],
        "stderr_tail": stderr[-1000:],
        "elapsed_s": time.time() - started,
    }


def wait_idle(request, session_id, baseline_message_id, timeout=900):
    deadline = time.time() + timeout
    saw_active = False
    while time.time() < deadline:
        runtime = request("GET", f"/session/{session_id}/runtime", timeout=10) or {}
        status = runtime.get("run_status")
        if status and status != "idle":
            saw_active = True
        if baseline_message_id:
            anchor = urllib.parse.quote(baseline_message_id, safe="")
            path = f"/session/{session_id}/message?after={anchor}&limit=200"
        else:
            path = f"/session/{session_id}/message?limit=200"
        new_messages = request("GET", path, timeout=15) or []
        completed_assistant = any(
            message.get("role") == "assistant" and message.get("finish") is not None
            for message in new_messages
        )
        # A recovery request can append a user message before Busy is visible.
        if status == "idle" and (saw_active or completed_assistant):
            return True
        time.sleep(0.5)
    return False


def wait_recoverable(request, session_id, timeout=60):
    deadline = time.time() + timeout
    while time.time() < deadline:
        protocol = request("GET", f"/session/{session_id}/recovery", timeout=10) or {}
        actions = {item.get("kind") for item in protocol.get("actions", [])}
        if protocol.get("status") == "recoverable" and "resume" in actions:
            return True
        time.sleep(0.5)
    return False


def ledger_anchor(ledger):
    generation = ledger.get("goal_generation", 0)
    checkpoints = [
        item.get("id")
        for item in ledger.get("verified", [])
        if item.get("goal_generation") == generation and not item.get("superseded_by")
    ]
    open_ids = [
        item.get("id")
        for item in ledger.get("open", [])
        if not item.get("closed_by_checkpoint_id")
    ]
    next_value = ledger.get("next") or {}
    goal = ledger.get("goal") or {}
    return {
        "revision": ledger.get("revision", 0),
        "goal_generation": generation,
        "goal": goal.get("statement"),
        "acceptance_criteria": goal.get("acceptance_criteria", []),
        "checkpoint_ids": checkpoints,
        "open_ids": open_ids,
        "next_statement": next_value.get("statement"),
        "status": ledger.get("status"),
    }


def continuity_packet(messages):
    for message in reversed(messages):
        packet = (message.get("metadata") or {}).get(
            "context_compaction_continuity_packet"
        )
        if packet:
            return packet
    return None


def packet_matches_anchor(packet, anchor):
    ledger = (packet or {}).get("task_ledger") or {}
    return (
        ledger.get("revision") == anchor.get("revision")
        and ledger.get("goal_generation") == anchor.get("goal_generation")
        and ledger.get("goal") == anchor.get("goal")
        and ledger.get("acceptance_criteria", []) == anchor.get("acceptance_criteria", [])
        and [item.get("id") for item in ledger.get("verified", [])]
        == anchor.get("checkpoint_ids", [])
        and [item.get("id") for item in ledger.get("open", [])]
        == anchor.get("open_ids", [])
        and ((ledger.get("next") or {}).get("statement")) == anchor.get("next_statement")
        and ledger.get("status") == anchor.get("status")
    )


def recovery_response_matches(response, anchor):
    return (
        response.get("recovery_ledger_revision") == anchor.get("revision")
        and response.get("recovery_checkpoint_ids", []) == anchor.get("checkpoint_ids", [])
        and response.get("recovery_open_ids", []) == anchor.get("open_ids", [])
        and response.get("recovery_next_statement") == anchor.get("next_statement")
    )


def add_compaction_history(request, session_id):
    messages = request("GET", f"/session/{session_id}/message?limit=200", timeout=15) or []
    index = 0
    while len(messages) < 10:
        request(
            "POST",
            f"/session/{session_id}/message",
            {"content": f"Evaluation continuity note {index}; no action required."},
            timeout=15,
        )
        index += 1
        messages = request("GET", f"/session/{session_id}/message?limit=200", timeout=15) or []


def run_workflow(
    command,
    task,
    session_id,
    directory,
    resume_pause_secs,
    *,
    request,
    run_command,
):
    workflow = task["workflow"]
    evidence = {
        "compaction_attempted": False,
        "compaction_succeeded": None,
        "ledger_restored_after_compaction": None,
        "continuity_packet_matches": None,
        "recovery_attempted": False,
        "recovery_anchor_matches": None,
        "resume_completed": None,
        "interruption_observed": None,
        "initial_process_exited_before_interrupt": False,
    }
    if workflow not in ("compaction", "resume"):
        return run_command(command), evidence

    if workflow == "compaction":
        add_compaction_history(request, session_id)
        first = run_command(command)
        before_ledger = request("GET", f"/session/{session_id}/task-ledger", timeout=15) or {}
        before_anchor = ledger_anchor(before_ledger)
        evidence["compaction_attempted"] = True
        compacted = request("POST", f"/session/{session_id}/compact", {}, timeout=30) or {}
        evidence["compaction_succeeded"] = bool(compacted.get("success"))
        after_ledger = request("GET", f"/session/{session_id}/task-ledger", timeout=15) or {}
        evidence["ledger_restored_after_compaction"] = after_ledger == before_ledger
        messages = request("GET", f"/session/{session_id}/message?limit=200", timeout=15) or []
        packet = continuity_packet(messages)
        governed = before_anchor["revision"] > 0
        evidence["continuity_packet_matches"] = (
            packet_matches_anchor(packet, before_anchor) if governed else None
        )
        evidence["recovery_attempted"] = True
        baseline_message_id = messages[-1].get("id") if messages else None
        response = request(
            "POST",
            f"/session/{session_id}/recovery/execute",
            {"action": "resume"},
            timeout=30,
        ) or {}
        evidence["recovery_anchor_matches"] = (
            recovery_response_matches(response, before_anchor) if governed else None
        )
        evidence["resume_completed"] = wait_idle(request, session_id, baseline_message_id)
        return first, evidence

    marker = os.path.join(directory, task["interrupt_marker"])
    started = time.time()
    process = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    marker_deadline = time.time() + 120
    while time.time() < marker_deadline and process.poll() is None and not os.path.exists(marker):
        time.sleep(0.25)
    if process.poll() is not None and not os.path.exists(marker):
        stdout, stderr = process.communicate()
        evidence["initial_process_exited_before_interrupt"] = True
        return process_result(process.returncode, stdout, stderr, started), evidence
    evidence["interruption_observed"] = os.path.exists(marker) and process.poll() is None
    try:
        request("POST", f"/session/{session_id}/abort", {}, timeout=15)
    except Exception:
        pass
    try:
        stdout, stderr = process.communicate(timeout=30)
        exit_code = process.returncode
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()
        exit_code = "timeout"
    before_ledger = request("GET", f"/session/{session_id}/task-ledger", timeout=15) or {}
    before_anchor = ledger_anchor(before_ledger)
    baseline_messages = request("GET", f"/session/{session_id}/message?limit=200", timeout=15) or []
    baseline_message_id = baseline_messages[-1].get("id") if baseline_messages else None
    time.sleep(resume_pause_secs)
    evidence["recovery_attempted"] = True
    if not wait_recoverable(request, session_id):
        evidence["resume_completed"] = False
        return process_result(exit_code, stdout, stderr, started), evidence
    response = request(
        "POST",
        f"/session/{session_id}/recovery/execute",
        {"action": "resume"},
        timeout=30,
    ) or {}
    governed = before_anchor["revision"] > 0
    evidence["recovery_anchor_matches"] = (
        recovery_response_matches(response, before_anchor) if governed else None
    )
    evidence["resume_completed"] = wait_idle(request, session_id, baseline_message_id)
    return process_result(exit_code, stdout, stderr, started), evidence
