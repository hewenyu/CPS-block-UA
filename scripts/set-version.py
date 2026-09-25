#!/usr/bin/env python3
"""Set the local build version from a validated vMAJOR.MINOR.PATCH[-prerelease] tag."""

import json
from pathlib import Path
import re
import sys
import tomllib


def main():
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python3 scripts/set-version.py v0.1.0[-rc.1]")
    tag = sys.argv[1]
    if not re.fullmatch(r"v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", tag):
        raise SystemExit("Invalid version tag; expected vMAJOR.MINOR.PATCH[-prerelease]")
    version = tag[1:]
    if "-" in version and any(
        part.isdigit() and len(part) > 1 and part.startswith("0")
        for part in version.split("-", 1)[1].split(".")
    ):
        raise SystemExit("Numeric prerelease identifiers cannot have leading zeros")

    cargo_path, lock_path, manifest_path = map(Path, ["Cargo.toml", "Cargo.lock", "plugin.json"])
    cargo_source = cargo_path.read_text()
    cargo = tomllib.loads(cargo_source)
    name = cargo["package"]["name"]
    cargo_source, changed = re.subn(r'^version = "[^"]+"$', f'version = "{version}"', cargo_source, count=1, flags=re.MULTILINE)
    assert changed == 1
    expected = cargo.copy()
    expected["package"] = {**cargo["package"], "version": version}
    assert tomllib.loads(cargo_source) == expected, "Only package.version may change"

    sections = lock_path.read_text().split("[[package]]")
    matches = 0
    for index in range(1, len(sections)):
        package = tomllib.loads(sections[index])
        if package["name"] == name and "source" not in package:
            sections[index], changed = re.subn(r'^version = "[^"]+"$', f'version = "{version}"', sections[index], count=1, flags=re.MULTILINE)
            assert changed == 1
            matches += 1
    assert matches == 1, "Expected one local package in Cargo.lock"

    manifest = json.loads(manifest_path.read_text())
    manifest["version"] = version
    # Validate everything before writing; dependency versions and checksums stay unchanged.
    cargo_path.write_text(cargo_source)
    lock_path.write_text("[[package]]".join(sections))
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"Build version: {version}")


if __name__ == "__main__":
    main()
