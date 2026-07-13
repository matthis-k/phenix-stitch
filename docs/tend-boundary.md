# Stitch–Tend boundary

Stitch and Tend have separate authorities.

## Stitch owns orchestration

Stitch discovers workspace members, derives dependency edges from each member's
`flake.lock`, selects graph closure, orders repositories, and coordinates
multi-repository mutations such as commits, input updates, and pushes.

## Tend owns repository-local verification

Tend resolves and executes the checks declared by the repository being visited.
Stitch never reconstructs Tend tasks or links Tend's planning library. It invokes
the `tend` executable in the repository directory through this contract:

```text
tend check --profile <profile> --context <context>
           [--base <git-base> --head <git-head>]
           [--file <path>]...
```

Both `profile` and `context` are mandatory at every Stitch execution boundary.
A profile defines phase, file selection, and logical tasks. A context defines the
allowed implementation and runtime capabilities.

## MCP runtime ownership

`phenix-mcp-core` is local workspace infrastructure used by `stitch-mcp`. It is
not part of Tend and does not create a library dependency between the two tools.
It may be extracted into an independent infrastructure package later if another
consumer needs the same protocol runtime.

## Runtime packaging

The Nix `stitch` and `stitch-mcp` applications wrap their unwrapped Rust binaries
with Tend, Git, Nix, and Jujutsu on `PATH`. The Rust workspace therefore has no
direct dependency on the Tend runner crate. `stitch-unwrapped` and
`stitch-mcp-unwrapped` remain available for low-level packaging.

## Standard profiles

Stitch's own repository defines these Tend v2 profiles:

- `git-hook` with `local`
- `manual` with `local`
- `full` with `local` or `nix-sandbox`
- `pre-push` with `local`
- `ci` with `local`
- `fix` with `local`

Installed hooks follow the same boundary. Workspace-root hooks call `stitch
verify` so graph selection is preserved; repository-local hooks call Tend
directly.
