#!/usr/bin/env python3
"""Drive a running corntchap server through the official MCP Python SDK (Streamable HTTP, 2026-07-28).
Usage: CORNTCHAP_URL=http://localhost:8080 CORNTCHAP_TOKEN=<agent token> PROJECT=DEMO scripts/mcp_check.py
Needs: pip install mcp   (just mcp-check sets up a venv). Creates one task, claims it, ticks, completes, and checks fencing."""
import asyncio, json, os, sys
import httpx2
from mcp import Client
from mcp.client.streamable_http import streamable_http_client

URL = os.environ.get("CORNTCHAP_URL", "http://localhost:8080").rstrip("/")
TOKEN = os.environ["CORNTCHAP_TOKEN"]
PROJECT = os.environ.get("PROJECT", "DEMO")

def body(r):
    return json.loads(r.content[0].text)

async def main():
    http = httpx2.AsyncClient(headers={"Authorization": f"Bearer {TOKEN}"}, timeout=60)
    async with Client(streamable_http_client(f"{URL}/mcp", http_client=http)) as c:
        names = [t.name for t in (await c.list_tools()).tools]
        assert {"claim_next", "complete", "check_item", "wait_for_work"} <= set(names), names
        created = body(await c.call_tool("create_task", {"project": PROJECT, "title": "mcp_check", "items": "one\ntwo"}))
        claim = body(await c.call_tool("claim_task", {"id": created["id"]}))
        assert "task" in claim, claim
        tid, ep = claim["task"]["id"], claim["lease"]["epoch"]
        for pos in (1, 2):
            assert not (await c.call_tool("check_item", {"id": tid, "epoch": ep, "position": pos, "done": True})).is_error
        stale = await c.call_tool("check_item", {"id": tid, "epoch": ep + 99, "position": 1, "done": True})
        assert stale.is_error and "lease_lost" in stale.content[0].text, stale
        assert not (await c.call_tool("complete", {"id": tid, "epoch": ep, "result": "mcp_check ok"})).is_error
        again = await c.call_tool("complete", {"id": tid, "epoch": ep, "result": "twice"})
        assert again.is_error and "lease_lost" in again.content[0].text
        print(f"MCP OK: {len(names)} tools, {tid} claimed/ticked/completed, fencing enforced")

asyncio.run(main())
