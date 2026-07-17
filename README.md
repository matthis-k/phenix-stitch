# Stitch

Stitch derives a repository dependency graph from each repository's `flake.lock` and optional root-local `.stitch.json` declaration, then executes an arbitrary command over a selected closure in a deterministic order.

```sh
stitch graph verify . --strict
stitch exec --all --order providers-first -- git status --short
stitch exec --changed --closure downstream --order providers-first -- devenv test
stitch exec --all --order consumers-first --apply -- git pull --ff-only
```

An edge is stored as `consumer -> provider`. Selection (`self`, `upstream`, `downstream`, `connected`, `all`) is independent from execution order (`stable`, `providers-first`, `consumers-first`). Mutating commands require `--apply`. Execution stops at the first failed repository unless `--keep-going` is supplied.

Repository-local explicit graph metadata is optional:

```json
{
  "role": "producer",
  "layer": 2,
  "dependencies": ["phenix-pins"]
}
```

Stitch does not know how a repository is tested, formatted, committed, or published. Compose those operations at the command line, for example `stitch exec ... -- devenv test`.

## Managed workspaces

A workspace root may commit `.stitch-workspace.json` as a discovery policy without committing a repository inventory:

```json
{
  "owner": "matthis-k",
  "repository_pattern": "phenix-*",
  "search_roots": ["repos"]
}
```

Stitch derives the desired repositories from all matching GitHub nodes in the root `flake.lock`, including transitive inputs.

```sh
# Preview missing clones.
stitch workspace populate .

# Clone missing repositories into the configured search root.
stitch workspace populate . --apply

# Preview obsolete Stitch-managed clones.
stitch workspace clean .

# Populate and prune obsolete managed clones.
stitch workspace sync . --prune --apply
```

Cleanup is deliberately conservative. Stitch only removes repositories that it cloned and recorded in its XDG state. Unknown directories are untouched, changed remotes are blocked, and dirty repositories require both `--apply` and `--force`.

## Repository maintenance

The repository uses standalone devenv tasks. The project flake remains independent from devenv. CI executes the same deterministic maintenance graph as local development.

```sh
devenv test
devenv tasks run maintenance:check
devenv tasks run maintenance:fix
```
