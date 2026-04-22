#!/usr/bin/env python3
import argparse
import json
import os
import pty
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


ANSWERS = {
    "goal": "Exercise the /autoresearch command preflight and reply loop.",
    "scope": "crates/rocode-server/**, crates/rocode-cli/**, crates/rocode-tui/**",
    "metric": "The command resubmits with all missing fields populated.",
    "verify": "cargo build -p rocode-cli -p rocode-server -p rocode-tui",
}


class SmokeFailure(RuntimeError):
    pass


class PtyProcess:
    def __init__(self, argv, cwd, env):
        self.master_fd, slave_fd = pty.openpty()
        self.proc = subprocess.Popen(
            argv,
            cwd=str(cwd),
            env=env,
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            start_new_session=True,
            close_fds=True,
        )
        os.close(slave_fd)
        self._buffer = bytearray()
        self._stop = threading.Event()
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()

    def _read_loop(self):
        while not self._stop.is_set():
            try:
                chunk = os.read(self.master_fd, 4096)
            except OSError:
                break
            if not chunk:
                break
            self._buffer.extend(chunk)

    def write(self, text):
        os.write(self.master_fd, text.encode())

    def output(self):
        return self._buffer.decode(errors="ignore")

    def terminate(self):
        self._stop.set()
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            deadline = time.time() + 5
            while time.time() < deadline:
                if self.proc.poll() is not None:
                    break
                time.sleep(0.1)
            if self.proc.poll() is None:
                try:
                    os.killpg(self.proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        try:
            os.close(self.master_fd)
        except OSError:
            pass
        self._reader.join(timeout=1)


def http_json(url, method="GET", payload=None, timeout=10):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode()
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, method=method, headers=headers)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        body = response.read()
    if not body:
        return None
    return json.loads(body.decode())


def wait_until(description, predicate, timeout=30, interval=0.2):
    deadline = time.time() + timeout
    last_error = None
    while time.time() < deadline:
        try:
            value = predicate()
            if value:
                return value
        except Exception as error:  # noqa: BLE001
            last_error = error
        time.sleep(interval)
    if last_error is not None:
        raise SmokeFailure(f"Timed out waiting for {description}: {last_error}")
    raise SmokeFailure(f"Timed out waiting for {description}")


def wait_for_health(base_url, timeout=30):
    def predicate():
        try:
            response = http_json(f"{base_url}/health", timeout=3)
        except Exception:  # noqa: BLE001
            return None
        return response if response and response.get("status") == "ok" else None

    return wait_until("server health", predicate, timeout=timeout)


def list_questions(base_url):
    return http_json(f"{base_url}/question") or []


def get_session(base_url, session_id):
    return http_json(f"{base_url}/session/{session_id}")


def session_metadata(base_url, session_id):
    return get_session(base_url, session_id).get("metadata") or {}


def wait_for_new_question(base_url, seen_ids, timeout=30):
    def predicate():
        for question in list_questions(base_url):
            if question["id"] not in seen_ids:
                return question
        return None

    return wait_until("new pending question", predicate, timeout=timeout)


def wait_for_question_resolved(base_url, question_id, timeout=30):
    def predicate():
        return all(question["id"] != question_id for question in list_questions(base_url))

    wait_until(f"question {question_id} to resolve", predicate, timeout=timeout)


def merge_pending_command_arguments(pending):
    parts = []
    raw = (pending.get("rawArguments") or "").strip()
    if raw:
        parts.append(raw)
    for field in pending.get("missingFields") or []:
        value = ANSWERS.get(field, "").strip()
        if not value:
            continue
        parts.append(f"--{field}")
        if field == "scope":
            parts.extend(part for part in value.splitlines() if part.strip())
        else:
            escaped = value.replace("\\", "\\\\").replace('"', '\\"')
            parts.append(f'"{escaped}"' if any(ch.isspace() for ch in value) else value)
    return " ".join(parts).strip()


def run_protocol_smoke(base_url):
    session = http_json(f"{base_url}/session", method="POST", payload={})
    response = http_json(
        f"{base_url}/session/{session['id']}/prompt",
        method="POST",
        payload={"message": "/autoresearch"},
    )
    if response.get("status") != "awaiting_user":
        raise SmokeFailure(f"protocol smoke expected awaiting_user, got {response}")
    question_id = response.get("pending_question_id")
    questions = {question["id"]: question for question in list_questions(base_url)}
    if question_id not in questions:
        raise SmokeFailure("protocol smoke question was not registered")
    pending = session_metadata(base_url, session["id"]).get("pending_command_invocation")
    if not pending:
        raise SmokeFailure("protocol smoke missing pending_command_invocation metadata")
    answers = [[ANSWERS[field]] for field in pending.get("missingFields") or []]
    http_json(
        f"{base_url}/question/{question_id}/reply",
        method="POST",
        payload={"answers": answers},
    )
    response = http_json(
        f"{base_url}/session/{session['id']}/prompt",
        method="POST",
        payload={
            "command": pending["command"],
            "arguments": merge_pending_command_arguments(pending),
        },
    )
    if response.get("status") != "accepted":
        raise SmokeFailure(f"protocol smoke expected accepted, got {response}")
    metadata = session_metadata(base_url, session["id"])
    if metadata.get("pending_command_invocation") is not None:
        raise SmokeFailure("protocol smoke expected pending_command_invocation to clear")
    return session["id"]


def drive_cli_smoke(base_url, repo_root, rocode_bin):
    seen = {question["id"] for question in list_questions(base_url)}
    env = os.environ.copy()
    env["ROCODE_SERVER_URL"] = base_url
    env.setdefault("OPENAI_API_KEY", "smoke-test-key")
    cli = PtyProcess(
        [str(rocode_bin), "run", "--interactive-mode", "compact"],
        cwd=repo_root,
        env=env,
    )
    try:
        time.sleep(1.0)
        cli.write("/autoresearch\n")
        question = wait_for_new_question(base_url, seen, timeout=30)
        time.sleep(0.5)
        cli.write(ANSWERS["goal"] + "\n")
        cli.write(ANSWERS["scope"] + "\n")
        cli.write(ANSWERS["metric"] + "\n")
        cli.write(ANSWERS["verify"] + "\n")
        wait_for_question_resolved(base_url, question["id"], timeout=30)
        wait_until(
            "CLI pending command metadata clear",
            lambda: session_metadata(base_url, question["session_id"]).get(
                "pending_command_invocation"
            )
            is None,
            timeout=30,
        )
        metadata = session_metadata(base_url, question["session_id"])
        output = cli.output()
        if "What should autoresearch improve?" not in output:
            raise SmokeFailure("CLI smoke did not render the autoresearch question prompt")
        return {
            "session_id": question["session_id"],
            "metadata": metadata,
            "output_excerpt": output[-600:],
        }
    finally:
        cli.terminate()


def drive_tui_smoke(base_url, repo_root, rocode_bin):
    seen = {question["id"] for question in list_questions(base_url)}
    env = os.environ.copy()
    env["ROCODE_TUI_PROMPT"] = "/autoresearch"
    tui = PtyProcess(
        [str(rocode_bin), "attach", base_url, "--dir", str(repo_root)],
        cwd=repo_root,
        env=env,
    )
    try:
        question = wait_for_new_question(base_url, seen, timeout=30)
        resolved = False
        for attempt in range(4):
            time.sleep(7.0 if attempt == 0 else 5.0)
            for key in ("goal", "scope", "metric", "verify"):
                tui.write(ANSWERS[key])
                tui.write("\r")
                time.sleep(1.0)
            try:
                wait_for_question_resolved(base_url, question["id"], timeout=8)
                resolved = True
                break
            except SmokeFailure:
                continue
        if not resolved:
            raise SmokeFailure(f"Timed out waiting for question {question['id']} to resolve")
        wait_until(
            "TUI pending command metadata clear",
            lambda: session_metadata(base_url, question["session_id"]).get(
                "pending_command_invocation"
            )
            is None,
            timeout=30,
        )
        metadata = session_metadata(base_url, question["session_id"])
        output = tui.output()
        return {
            "session_id": question["session_id"],
            "metadata": metadata,
            "output_excerpt": output[-600:],
        }
    finally:
        tui.terminate()


def run_web_smoke(base_url, repo_root):
    script = repo_root / "crates" / "rocode-server" / "web" / "scripts" / "autoresearch-smoke.mjs"
    env = os.environ.copy()
    env["ROCODE_BASE_URL"] = base_url
    completed = subprocess.run(
        ["node", str(script)],
        cwd=str(script.parent.parent),
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if completed.returncode != 0:
        raise SmokeFailure(
            f"Web smoke failed with exit {completed.returncode}\nSTDOUT:\n{completed.stdout}\nSTDERR:\n{completed.stderr}"
        )
    return completed.stdout.strip()


def maybe_start_server(base_url, rocode_bin):
    port = urllib.parse.urlparse(base_url).port
    if port is None:
        raise SmokeFailure(f"base url must include a port: {base_url}")
    env = os.environ.copy()
    env["ROCODE_FRONTEND_SMOKE_SKIP_EXECUTION"] = "1"
    server = subprocess.Popen(
        [str(rocode_bin), "serve", "--port", str(port)],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        wait_for_health(base_url, timeout=30)
    except Exception:
        stdout, stderr = server.communicate(timeout=5)
        raise SmokeFailure(f"failed to start smoke server\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}")
    return server


def stop_server(server):
    if server is None:
        return
    if server.poll() is None:
        server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)


def main():
    repo_root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:4100")
    parser.add_argument("--start-server", action="store_true")
    parser.add_argument(
        "--rocode-bin",
        default=str(repo_root.parent / "target" / "debug" / "rocode"),
    )
    parser.add_argument("--skip-web", action="store_true")
    parser.add_argument("--skip-cli", action="store_true")
    parser.add_argument("--skip-tui", action="store_true")
    args = parser.parse_args()

    rocode_bin = Path(args.rocode_bin)
    if not rocode_bin.exists():
        raise SmokeFailure(f"rocode binary not found: {rocode_bin}")

    server = maybe_start_server(args.base_url, rocode_bin) if args.start_server else None
    try:
        wait_for_health(args.base_url, timeout=30)
        summary = []
        print("Running protocol smoke...", flush=True)
        protocol_session_id = run_protocol_smoke(args.base_url)
        summary.append(f"protocol: session {protocol_session_id}")
        if not args.skip_cli:
            print("Running CLI smoke...", flush=True)
            cli_result = drive_cli_smoke(args.base_url, repo_root, rocode_bin)
            summary.append(f"cli: session {cli_result['session_id']}")
        if not args.skip_tui:
            print("Running TUI smoke...", flush=True)
            tui_result = drive_tui_smoke(args.base_url, repo_root, rocode_bin)
            summary.append(f"tui: session {tui_result['session_id']}")
        if not args.skip_web:
            print("Running Web smoke...", flush=True)
            web_result = run_web_smoke(args.base_url, repo_root)
            summary.append(f"web: {web_result}")
        print("Autoresearch E2E smoke passed")
        for index, line in enumerate(summary, start=1):
            print(f"{index}. {line}")
    finally:
        stop_server(server)


if __name__ == "__main__":
    try:
        main()
    except SmokeFailure as error:
        print(f"Autoresearch E2E smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors="ignore")
        print(
            f"Autoresearch E2E smoke failed: HTTP {error.code} {error.reason}\n{body}",
            file=sys.stderr,
        )
        sys.exit(1)
