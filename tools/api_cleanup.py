#!/usr/bin/env python3
"""Collapse Stitch-owned persisted formats to their current unversioned shape.

This repository is pre-release. Old serialized shapes are intentionally not
accepted or migrated; Git history is the only archive of superseded APIs.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text()
    if old not in text:
        raise SystemExit(f"expected text not found in {path}: {old!r}")
    target.write_text(text.replace(old, new))


def regex_replace(path: str, pattern: str, replacement: str) -> None:
    target = ROOT / path
    text = target.read_text()
    updated, count = re.subn(pattern, replacement, text, flags=re.MULTILINE)
    if count == 0:
        raise SystemExit(f"pattern did not match in {path}: {pattern!r}")
    target.write_text(updated)


# Workspace configuration/state are always the current shape.
replace("crates/stitch/src/model.rs", "    pub version: u32,\n", "")
replace("crates/stitch/src/workspace.rs", "    pub version: u32,\n", "")
replace("crates/stitch/src/workspace.rs", "        version: 2,\n", "")
replace("crates/stitch/src/graph/inventory.rs", "            version: 2,\n", "")

# Changesets are not a multi-version protocol.
replace("crates/stitch/src/model.rs", "    pub version: u32,\n", "")
replace("crates/stitch/src/changeset/new.rs", "        version: 1,\n", "")

# Recipes accept exactly the current object shape.
replace("crates/stitch/src/recipe.rs", "    pub version: u32,\n", "")
replace(
    "crates/stitch/src/recipe.rs",
    "        return Ok(RecipeCollection {\n            version: 1,\n            recipes: Vec::new(),\n        });",
    "        return Ok(RecipeCollection {\n            recipes: Vec::new(),\n        });",
)
replace(
    "crates/stitch/src/recipe.rs",
    "    if collection.version < 1 {\n        return Err(format!(\"Unsupported recipe version {}\", collection.version));\n    }\n",
    "",
)

# Work-loop wallets likewise expose only the current schema.
replace("crates/stitch/src/workloop.rs", "    pub schema_version: u32,\n", "")
regex_replace(
    "crates/stitch/src/workloop.rs",
    r"^\s*schema_version:\s*\d+,\n",
    "",
)

# Topology metadata is classification data, not a version negotiation surface.
replace("crates/stitch/src/graph/inventory.rs", "    version: u32,\n", "")
regex_replace(
    "crates/stitch/src/graph/inventory.rs",
    r'^\s*"version":\s*\d+,\n',
    "",
)

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
