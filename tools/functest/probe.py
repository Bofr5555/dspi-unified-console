#!/usr/bin/env python3
"""Call the app's MCP tools directly, as an agent would, without a model: for
chasing one problem in an app that `functest.py up` left running.

    probe.py NAME tool '{"json": "args"}' [tool '{...}' ...]

Prints each answer. The relay pairs as any agent; the harness answers (it must
be allowed once: `functest.py ask` did, or this allows it through the socket).
"""
import json
import subprocess
import sys
import threading
import time

from functest import CONTAINER_WORK, Run


class Agent:
    def __init__(self, name, client="probe"):
        self.run = Run(name)
        self.p = subprocess.Popen(
            ["podman", "exec", "-i", self.run.container, f"{CONTAINER_WORK}/build-functest/uconsole-for-dspi", "--mcp"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.n = 0
        self.last_news = ""
        # -- WP ODEC -- every answer an agent got, for the invariant that it never reads
        # that anything is simulated (checks.py)
        self.log = open(self.run.dir / f"mcp-{client}.jsonl", "a")
        threading.Thread(target=self.allow, daemon=True).start()
        self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": client, "version": "0"}})
        self.p.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        self.p.stdin.flush()

    def allow(self):
        for _ in range(40):
            try:
                if self.run.ctl.get("AiController", "pairing"):
                    self.run.ctl.call("AiController", "answerPairing", True)
            except (OSError, RuntimeError):
                pass
            time.sleep(0.5)

    def rpc(self, method, params):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            self.log.write(line)  # -- WP ODEC --
            self.log.flush()
            m = json.loads(line)
            if m.get("id") == self.n:
                return m

    def call(self, tool, **args):
        r = self.rpc("tools/call", {"name": tool, "arguments": args})
        if "error" in r:
            return "RPC ERROR", r["error"]
        r = r["result"]
        # -- WP INTF -- a success's first text is its structure; AIVIS adds the user's
        # changes as one more text after it, which is no JSON
        texts = [c.get("text", "") for c in r.get("content", [])]
        text = texts[0] if "structuredContent" in r and texts else "\n".join(texts)
        self.last_news = "\n".join(texts[1:])  # -- WP ODEC -- the user's news after the result
        return ("ERR" if r.get("isError") else "ok"), text


    # -- WP ODEC --
    def read_all(self):
        """What an agent can read besides tool results: the tool list, every resource, every
        prompt (logged like any answer). The number of items read."""
        n = 0
        for method, key in (("tools/list", "tools"), ("resources/list", "resources"),
                            ("resources/templates/list", "resourceTemplates"), ("prompts/list", "prompts")):
            r = self.rpc(method, {})
            items = (r.get("result") or {}).get(key) or []
            n += len(items)
            for it in items:
                if method == "resources/list":
                    self.rpc("resources/read", {"uri": it["uri"]})
                elif method == "prompts/list":
                    args = {a["name"]: "1" for a in it.get("arguments") or [] if a.get("required")}
                    self.rpc("prompts/get", {"name": it["name"], "arguments": args})
        return n


def main():
    a = Agent(sys.argv[1])
    for tool, args in zip(sys.argv[2::2], sys.argv[3::2]):
        kind, text = a.call(tool, **json.loads(args))
        print(f"{tool} {args}\n  {kind}: {str(text)[:1500]}")


if __name__ == "__main__":
    main()
