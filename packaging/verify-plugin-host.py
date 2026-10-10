"""Verify that a fresh Codex session can call the installed plugin."""

import json
import os
import queue
import threading
import time
import tomllib
from pathlib import Path
import shutil
import subprocess
import tempfile


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    executable = shutil.which("codex")
    if not executable:
        raise RuntimeError("Codex CLI was not found.")
    artifact = root / "dist/plugin-host-verification.json"
    with tempfile.TemporaryDirectory(prefix="cash-ledger-host-check-") as directory:
        database = Path(directory) / "ledger.sqlite3"
        environment = dict(os.environ, CASH_REGISTER_DB=str(database), CASH_LEDGER_UPDATE_PREFERENCES=str(Path(directory) / "updates.json"))
        command = [executable, "app-server"]
        config_path = Path.home() / ".codex/config.toml"
        if config_path.exists():
            config = tomllib.loads(config_path.read_text(encoding="utf-8"))
            for server_name in config.get("mcp_servers", {}):
                command.extend(["-c", f'mcp_servers.{server_name}.enabled=false'])
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", env=environment)
        diagnostics = []

        def read_errors():
            for line in process.stderr:
                if "cash-ledger" in line and "http" not in line and "token" not in line.lower():
                    diagnostics.append(line.strip())

        threading.Thread(target=read_errors, daemon=True).start()
        messages = queue.Queue()

        def read_output():
            for line in process.stdout:
                try:
                    messages.put(json.loads(line))
                except json.JSONDecodeError:
                    pass
            messages.put({"id": -1, "error": "Codex exited before responding"})

        threading.Thread(target=read_output, daemon=True).start()
        sequence = 0

        def request(method, params):
            nonlocal sequence
            print(f"Checking Codex {method}...", flush=True)
            sequence += 1
            process.stdin.write(json.dumps({"id": sequence, "method": method, "params": params}) + "\n")
            process.stdin.flush()
            deadline = time.monotonic() + 150
            while True:
                message = messages.get(timeout=max(0.1, deadline - time.monotonic()))
                if message.get("id") == -1:
                    raise RuntimeError("Codex exited before responding")
                if "cash-ledger" in json.dumps(message) and "http" not in json.dumps(message) and "id" not in message:
                    diagnostics.append(json.dumps(message))
                if message.get("id") == sequence:
                    if "error" in message:
                        raise RuntimeError(f"Codex rejected {method}: {message['error'].get('message', 'unknown error')}")
                    return message["result"]

        try:
            request("initialize", {"clientInfo": {"name": "cash_ledger_verifier", "version": "0.1.0"}, "capabilities": {"experimentalApi": True}})
            process.stdin.write('{"method":"initialized"}\n')
            process.stdin.flush()
            thread = request("thread/start", {"cwd": str(root), "ephemeral": True, "approvalPolicy": "never", "sandbox": "workspace-write"})["thread"]["id"]
            statuses = []
            cursor = None
            while True:
                page = request("mcpServerStatus/list", {"threadId": thread, "limit": 100, "cursor": cursor})
                statuses.extend(page["data"])
                cursor = page.get("nextCursor")
                if not cursor:
                    break
            candidates = [item for item in statuses if "cash" in item["name"].lower()]
            deadline = time.monotonic() + 90
            while len(candidates) == 1 and not candidates[0]["tools"] and time.monotonic() < deadline:
                time.sleep(2)
                page = request("mcpServerStatus/list", {"threadId": thread, "limit": 100})
                candidates = [item for item in page["data"] if "cash" in item["name"].lower()]
            if len(candidates) != 1 or "status" not in candidates[0]["tools"]:
                raise RuntimeError(f"Cash Ledger discovery failed: {[(item['name'], list(item['tools'])) for item in candidates]}; {diagnostics}")
            server = candidates[0]["name"]
            payload = request("mcpServer/tool/call", {"threadId": thread, "server": server, "tool": "status", "arguments": {}})
            artifact.parent.mkdir(exist_ok=True)
            artifact.write_text(json.dumps({"server": server, "tool_count": len(candidates[0]["tools"]), "result": payload}, indent=2), encoding="utf-8")
        finally:
            process.terminate()
            process.wait(timeout=15)
        assert not payload.get("isError", False), payload
        status = payload.get("structuredContent") or payload.get("structured_content")
        if status is None:
            status = json.loads(next(block["text"] for block in payload["content"] if block.get("type") == "text"))
        assert status["setup_required"] is True and status["authenticated"] is False
        assert Path(status["database_path"]) == database
        print("Codex discovered the installed Cash Ledger tools and called status successfully.")
        print(f"Verification trace: {artifact}")


if __name__ == "__main__":
    main()
