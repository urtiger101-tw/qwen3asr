"""Live acceptance with the official MCP client SDK (developer-only dependency)."""
from __future__ import annotations

import argparse
import asyncio
import json
import os
import time
import uuid
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client


def unpack(result):
    assert result.content and result.content[0].type == "text", result
    data = json.loads(result.content[0].text)
    assert not result.isError, data
    return data


async def validate(executable: Path, output: Path, sample: Path, long_sample: Path):
    output = output / ("run-" + uuid.uuid4().hex[:12])
    output.mkdir(parents=True, exist_ok=True)
    params = StdioServerParameters(command=str(executable), args=["mcp"], env=dict(os.environ))
    report = {}
    async with stdio_client(params) as (read, write), ClientSession(read, write) as session:
        initialized = await session.initialize()
        report["server"] = initialized.serverInfo.model_dump()
        listing = await session.list_tools()
        report["tools"] = [tool.name for tool in listing.tools]
        assert len(report["tools"]) == 6
        info = unpack(await session.call_tool("info", {}))
        assert info["result"]["runtime_ready"], info
        assert info["result"]["python_required"] is False, info
        report["runtime"] = info["result"]
        models = unpack(await session.call_tool("models_status", {}))
        assert sum(model["downloaded"] for model in models["result"]) >= 2
        started = unpack(await session.call_tool("transcription_start", {
            "inputs": [str(sample)], "output_dir": str(output / "recognition"),
            "formats": ["srt", "json"], "offline": True, "traditional": True,
        }))
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            status = unpack(await session.call_tool("transcription_status", {"job_id": started["job_id"]}))
            if status["status"] in ("succeeded", "failed", "cancelled"):
                break
            await asyncio.sleep(1)
        assert status["status"] == "succeeded", status
        assert status["exit_code"] == 0
        for path in status["result"]["artifacts"].values():
            assert Path(path).is_file(), path
        report["recognition"] = status

        # Cancel a live Rust CLI + native C++ worker tree, then ensure the queue advances.
        import psutil
        cancelled = unpack(await session.call_tool("transcription_start", {
            "inputs": [str(long_sample)], "output_dir": str(output / "cancelled"),
            "device": "cpu", "offline": True,
        }))
        queued = unpack(await session.call_tool("transcription_start", {
            "inputs": [str(sample)], "output_dir": str(output / "queued"),
            "formats": ["txt"], "timestamps": "none", "offline": True,
        }))
        assert queued["status"] == "queued", queued
        tracked = set()
        observed_names = set()
        for _ in range(50):
            for child in psutil.Process().children(recursive=True):
                try:
                    if child.name().lower() == "qwen3asr-worker.exe":
                        for process in [child, *child.children(recursive=True)]:
                            tracked.add((process.pid, process.create_time()))
                            observed_names.add(process.name())
                except psutil.Error:
                    pass
            if tracked:
                break
            await asyncio.sleep(0.2)
        assert tracked, "No actual inference backend was observed before cancellation"
        assert "qwen3asr-worker.exe" in {name.lower() for name in observed_names}
        before = time.monotonic()
        stopped = unpack(await session.call_tool("transcription_cancel", {"job_id": cancelled["job_id"]}))
        assert stopped["cancelled"]
        assert time.monotonic() - before < 10, "Cancellation blocked on orphan pipe readers"
        for pid, created in tracked:
            try:
                assert psutil.Process(pid).create_time() != created, f"Orphan inference process {pid}"
            except psutil.NoSuchProcess:
                pass
        # No status requests while the queued child runs: timer-driven queue must work.
        deadline = time.monotonic() + 120
        expected = output / "queued" / (sample.stem + ".txt")
        while not expected.exists() and time.monotonic() < deadline:
            await asyncio.sleep(1)
        queued_status = unpack(await session.call_tool("transcription_status", {"job_id": queued["job_id"]}))
        for _ in range(20):
            if queued_status["status"] == "succeeded":
                break
            await asyncio.sleep(0.2)
            queued_status = unpack(await session.call_tool("transcription_status", {"job_id": queued["job_id"]}))
        assert queued_status["status"] == "succeeded", queued_status
        report["cancellation"] = {
            "cancelled_job": stopped, "terminated_processes": len(tracked),
            "observed_process_names": sorted(observed_names),
        }
        report["queue"] = queued_status
    (output / "mcp-acceptance.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"status": "PASS", "tools": report["tools"], "report": str(output / "mcp-acceptance.json")}, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--sample", type=Path, required=True)
    parser.add_argument("--long-sample", type=Path, required=True)
    options = parser.parse_args()
    asyncio.run(validate(options.exe.resolve(), options.output.resolve(), options.sample.resolve(), options.long_sample.resolve()))
