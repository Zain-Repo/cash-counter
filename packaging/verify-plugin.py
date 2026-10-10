"""Validate a built Cash Ledger plugin and exercise its stdio discovery."""

from __future__ import annotations

import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile


def main() -> None:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[1] / "plugins/cash-ledger"
    manifest = json.loads((root / "plugin.json").read_text(encoding="utf-8-sig"))
    overlay = json.loads((root / ".codex-plugin/plugin.json").read_text(encoding="utf-8-sig"))
    assert manifest["name"] == overlay["name"] == "cash-ledger"
    assert root.name == manifest["name"] or (root.parent.name == manifest["name"] and root.name == manifest["version"])
    assert manifest["version"] == overlay["version"]
    interface = manifest["extensions"]["com.openai"]["interface"]
    assert interface == overlay["interface"]
    assert len(interface["shortDescription"]) <= 30
    for key in ("logo", "composerIcon"):
        asset = (root / interface[key]).resolve()
        assert asset.is_relative_to(root)
        data = asset.read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n"
        width, height = struct.unpack(">II", data[16:24])
        assert width == height and 48 <= width <= 4096 and len(data) <= 5 * 1024 * 1024
    for name, variable in (("mcp.json", "${PLUGIN_ROOT}"), (".mcp.json", "${PLUGIN_ROOT}")):
        config = json.loads((root / name).read_text(encoding="utf-8-sig"))
        server = config["mcpServers"]["cash-ledger"]
        command = server["command"].replace(variable, str(root))
        assert Path(command).is_file()
        assert "CASH_REGISTER_DB" in server["env_vars"]
    for path in root.rglob("*"):
        assert not path.is_symlink(), f"Symlink in plugin: {path}"
        if path.suffix in (".json", ".md"):
            assert "[TODO:" not in path.read_text(encoding="utf-8-sig"), f"Unfilled scaffold: {path}"
    with tempfile.TemporaryDirectory(prefix="cash-ledger-plugin-check-") as directory:
        database = Path(directory) / "test.sqlite3"
        environment = dict(os.environ, CASH_REGISTER_DB=str(database), CASH_LEDGER_UPDATE_PREFERENCES=str(Path(directory) / "updates.json"))
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "package-check", "version": "1"}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "status", "arguments": {}}},
        ]
        result = subprocess.run([str(root / "server/cash-ledger-mcp.exe")], input="".join(json.dumps(request) + "\n" for request in requests), capture_output=True, text=True, encoding="utf-8", env=environment, timeout=30, check=True)
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        assert [response["id"] for response in responses] == [1, 2, 3]
        tools = responses[1]["result"]["tools"]
        assert tools and len({tool["name"] for tool in tools}) == len(tools)
        assert not responses[2]["result"].get("isError", False), responses[2]
        status = responses[2]["result"]["structuredContent"]
        assert status["setup_required"] is True and status["authenticated"] is False
        assert Path(status["database_path"]) == database
        print(f"Validated Cash Ledger {manifest['version']}: {len(tools)} tools; startup, discovery, and status passed.")


if __name__ == "__main__":
    main()
