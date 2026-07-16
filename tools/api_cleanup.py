#!/usr/bin/env python3
"""Collapse Stitch-owned APIs to their single current shape.

This repository is pre-release. Old serialized and graph API generations are
intentionally not accepted or migrated; Git history is their only archive.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def rewrite(path: str, transform) -> None:
    target = ROOT / path
    text = target.read_text()
    updated = transform(text)
    if updated != text:
        target.write_text(updated)


def remove_literal(path: str, text: str) -> None:
    rewrite(path, lambda source: source.replace(text, ""))


def remove_regex(path: str, pattern: str) -> None:
    rewrite(path, lambda source: re.sub(pattern, "", source, flags=re.MULTILINE))


# Type definitions and primary constructors.
for path in [
    "crates/stitch/src/model.rs",
    "crates/stitch/src/workspace.rs",
    "crates/stitch/src/recipe.rs",
    "crates/stitch/src/graph/inventory.rs",
]:
    remove_literal(path, "    pub version: u32,\n")
    remove_literal(path, "    version: u32,\n")

remove_literal("crates/stitch/src/workloop.rs", "    pub schema_version: u32,\n")
remove_literal("crates/stitch/src/changeset/new.rs", "        version: 1,\n")
remove_literal("crates/stitch/src/recipe.rs", "            version: 1,\n")
remove_regex(
    "crates/stitch/src/recipe.rs",
    r'^\s*if collection\.version < 1 \{\n\s*return Err\(format!\("Unsupported recipe version \{\}", collection\.version\)\);\n\s*\}\n',
)

# Remove version fields from every Stitch-owned constructor and fixture.
for path in [
    "crates/stitch/src/workspace.rs",
    "crates/stitch/src/exec.rs",
    "crates/stitch/src/sync.rs",
    "crates/stitch/src/validate.rs",
    "crates/stitch/src/workloop.rs",
    "crates/stitch/src/graph/inventory.rs",
    "crates/stitch-cli/src/main.rs",
    "crates/stitch-mcp/src/tools.rs",
]:
    remove_regex(path, r"^\s*(?:schema_version|version):\s*\d+,\n")

# Remove version keys from current Stitch JSON examples and MCP output.
remove_regex("crates/stitch/src/graph/inventory.rs", r'^\s*"version":\s*\d+,\n')
remove_regex(
    "crates/stitch-mcp/src/tools.rs",
    r'^\s*"version":\s*cfg\.version,\n',
)

# Reject accidental reintroduction in Stitch-owned contracts and emitters.
owned_files = [
    ROOT / "crates/stitch/src/model.rs",
    ROOT / "crates/stitch/src/workspace.rs",
    ROOT / "crates/stitch/src/recipe.rs",
    ROOT / "crates/stitch/src/workloop.rs",
    ROOT / "crates/stitch/src/graph/inventory.rs",
    ROOT / "crates/stitch-cli/src/main.rs",
    ROOT / "crates/stitch-mcp/src/tools.rs",
]
for path in owned_files:
    text = path.read_text()
    if re.search(r"\b(?:schema_version|version)\s*:", text):
        raise SystemExit(f"Stitch-owned version field remains in {path.relative_to(ROOT)}")
