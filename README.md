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

Stitch does not know how a repository is tested, formatted, committed, published, cloned, pulled, or deleted. Compose repository operations outside Stitch.

## Workspace inventory

A workspace root may commit `.stitch-workspace.json` as a discovery policy without committing a mutable repository inventory:

```json
{
  "owner": "matthis-k",
  "repository_pattern": "phenix-*",
  "search_roots": ["repos"]
}
```

`workspace inventory` reports every matching GitHub repository in the complete root lock graph, including transitive inputs. It returns the desired local path and canonical remote but performs no mutation.

```sh
stitch workspace inventory .
stitch workspace inventory . --json
```

A separate workspace tool can consume this output to clone, update, or remove repositories while Stitch remains a read-only source of workspace structure.

## Repository maintenance

The repository uses standalone devenv tasks. The project flake remains independent from devenv. CI executes the same deterministic maintenance graph as local development.

```sh
devenv test
devenv tasks run maintenance:check
devenv tasks run maintenance:fix
```
