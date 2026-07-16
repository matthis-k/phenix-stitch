#!/usr/bin/env python3
"""Collapse Stitch-owned persisted formats to their current unversioned shape.

This repository is pre-release. Old serialized shapes are intentionally not
accepted or migrated; Git history is the only archive of superseded APIs.
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


# Workspace configuration/state and changesets always use the current shape.
remove_literal("crates/stitch/src/model.rs", "    pub version: u32,\n")
remove_literal("crates/stitch/src/workspace.rs", "    pub version: u32,\n")
remove_regex("crates/stitch/src/workspace.rs", r"^\s*version:\s*\d+,\n")
remove_literal("crates/stitch/src/changeset/new.rs", "        version: 1,\n")
remove_regex("crates/stitch/src/graph/inventory.rs", r"^\s*version:\s*\d+,\n")

# Recipes accept exactly the current object shape.
remove_literal("crates/stitch/src/recipe.rs", "    pub version: u32,\n")
remove_literal("crates/stitch/src/recipe.rs", "            version: 1,\n")
remove_regex(
    "crates/stitch/src/recipe.rs",
    r'^\s*if collection\.version < 1 \{\n\s*return Err\(format!\("Unsupported recipe version \{\}", collection\.version\)\);\n\s*\}\n',
)

# Work-loop wallets likewise expose only the current schema.
remove_literal("crates/stitch/src/workloop.rs", "    pub schema_version: u32,\n")
remove_regex("crates/stitch/src/workloop.rs", r"^\s*schema_version:\s*\d+,\n")

# Topology metadata is classification data, not a version negotiation surface.
remove_literal("crates/stitch/src/graph/inventory.rs", "    version: u32,\n")
remove_regex("crates/stitch/src/graph/inventory.rs", r'^\s*"version":\s*\d+,\n')

# Reject accidental reintroduction of Stitch-owned schema selectors.
owned_files = [
    ROOT / "crates/stitch/src/model.rs",
    ROOT / "crates/stitch/src/workspace.rs",
    ROOT / "crates/stitch/src/recipe.rs",
    ROOT / "crates/stitch/src/workloop.rs",
    ROOT / "crates/stitch/src/graph/inventory.rs",
]
for path in owned_files:
    text = path.read_text()
    if re.search(r"\b(?:schema_version|version)\s*:", text):
        raise SystemExit(f"Stitch-owned version field remains in {path.relative_to(ROOT)}")
