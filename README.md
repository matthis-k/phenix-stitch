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

## Repository maintenance

The repository uses standalone devenv tasks. The project flake remains independent from devenv.

```sh
devenv test
devenv tasks run maintenance:check
devenv tasks run maintenance:fix
```
