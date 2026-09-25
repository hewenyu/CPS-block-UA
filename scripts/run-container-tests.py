#!/usr/bin/env python3
"""Run cross-compiled Cargo test executables in the matching AMD64 runtime."""

import json
from pathlib import Path
import subprocess
import sys

root = Path(sys.argv[1]).resolve()
image = sys.argv[2]
executables = []
for line in (root / "target/x86_64-tests.json").read_text().splitlines():
    event = json.loads(line)
    if event.get("reason") == "compiler-message":
        print(event["message"].get("rendered", ""), file=sys.stderr)
    if (
        event.get("reason") == "compiler-artifact"
        and event.get("profile", {}).get("test")
        and event.get("executable")
    ):
        executable = event["executable"]
        if not executable.startswith("/work/target/x86_64-unknown-linux-gnu/release/"):
            raise SystemExit("Unexpected Cargo test executable path")
        executables.append(executable)

if not executables:
    raise SystemExit("No cross-compiled test executables found")

subprocess.run(
    [
        "docker", "run", "--rm", "--platform", "linux/amd64",
        "-v", f"{root}:/work:ro", "-w", "/work", image,
        "bash", "-euc", 'for executable in "$@"; do "$executable" --test-threads=1; done',
        "tests", *sorted(set(executables)),
    ],
    check=True,
)
