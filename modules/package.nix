{ inputs, lib, ... }: {
  perSystem =
    {
      pkgs,
      system,
      ...
    }:
    let
      source = lib.cleanSource ../.;
      tendPkg = inputs.phenix-tend.packages.${system}.default;

      rustToolchain = [
        pkgs.cargo
        pkgs.rustc
        pkgs.rustfmt
        pkgs.clippy
      ];

      stitchRuntime = [
        tendPkg
        pkgs.git
        pkgs.nix
        pkgs.jujutsu
      ];

      qualityRuntime = [
        pkgs.nixfmt
        pkgs.statix
        pkgs.deadnix
      ]
      ++ rustToolchain;

      cargoLock = {
        lockFile = ../Cargo.lock;
      };

      stitchCliUnwrapped = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch-unwrapped";
        version = "0.1.0";
        src = source;
        inherit cargoLock;
        cargoBuildFlags = "-p stitch-cli";
        nativeBuildInputs = [ pkgs.git ];
      };

      stitchMcpUnwrapped = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch-mcp-unwrapped";
        version = "0.1.0";
        src = source;
        inherit cargoLock;
        cargoBuildFlags = "-p stitch-mcp";
        nativeBuildInputs = [ pkgs.git ];
      };

      stitchCliPkg = pkgs.writeShellApplication {
        name = "stitch";
        runtimeInputs = stitchRuntime;
        text = ''
          exec ${stitchCliUnwrapped}/bin/stitch "$@"
        '';
      };

      stitchMcpPkg = pkgs.writeShellApplication {
        name = "stitch-mcp";
        runtimeInputs = stitchRuntime;
        text = ''
          exec ${stitchMcpUnwrapped}/bin/stitch-mcp "$@"
        '';
      };

      cargoDeps = stitchCliUnwrapped.cargoDeps or (throw "cargoDeps not found");

      tendGate =
        pkgs.runCommand "phenix-stitch-tend-gate"
          {
            nativeBuildInputs = [
              tendPkg
              pkgs.git
              pkgs.stdenv.cc
            ]
            ++ qualityRuntime;
            inherit cargoDeps;
            src = source;
          }
          ''
            export HOME=$TMPDIR/home
            mkdir -p "$HOME"
            export CARGO_HOME=$TMPDIR/cargo
            export CARGO_TARGET_DIR=$TMPDIR/target
            mkdir -p "$CARGO_HOME" "$CARGO_TARGET_DIR"

            cp -rT "$src" source
            chmod -R u+w source
            cd source

            cp -r ${cargoDeps}/.cargo .cargo
            ln -s ${cargoDeps} cargo-vendor-dir

            git init --quiet
            git add -A

            tend --root . check --profile full --context nix-sandbox

            touch "$out"
          '';

      tendFix = pkgs.writeShellApplication {
        name = "tend-fix";
        runtimeInputs = [
          tendPkg
          pkgs.git
        ];
        text = ''
          repo_root="$(git rev-parse --show-toplevel)"
          cd "$repo_root"

          mapfile -d '' staged_files < <(
            git diff --cached --name-only --diff-filter=ACMR -z
          )

          partially_staged=()
          for file in "''${staged_files[@]}"; do
            [[ -e "$file" ]] || continue
            if ! git diff --quiet -- "$file"; then
              partially_staged+=("$file")
            fi
          done

          if (( ''${#partially_staged[@]} > 0 )); then
            printf '%s\n' \
              'Cannot apply staged repairs to partially staged files.' \
              'Stage or stash their remaining changes first:' >&2
            printf '  %s\n' "''${partially_staged[@]}" >&2
            exit 1
          fi

          tend check --profile fix --context local

          if (( ''${#staged_files[@]} > 0 )); then
            git add -- "''${staged_files[@]}"
          fi

          exec tend check --profile git-hook --context local
        '';
      };

      tendVerify = pkgs.writeShellApplication {
        name = "tend-verify";
        runtimeInputs = [ tendPkg ];
        text = ''
          exec tend check --profile manual --context local "$@"
        '';
      };

      tendPrePush = pkgs.writeShellApplication {
        name = "tend-pre-push";
        runtimeInputs = [ tendPkg ];
        text = ''
          exec tend check --profile pre-push --context local "$@"
        '';
      };

      gitHooks = pkgs.runCommand "phenix-stitch-git-hooks" { } ''
        mkdir -p "$out"

        cat > "$out/pre-commit" <<'EOF'
        #!/usr/bin/env bash
        set -euo pipefail
        repo_root="$(${pkgs.git}/bin/git rev-parse --show-toplevel)"
        exec ${pkgs.nix}/bin/nix develop "$repo_root" --command tend-fix
        EOF

        cat > "$out/pre-push" <<'EOF'
        #!/usr/bin/env bash
        set -euo pipefail
        repo_root="$(${pkgs.git}/bin/git rev-parse --show-toplevel)"
        exec ${pkgs.nix}/bin/nix develop "$repo_root" --command tend-pre-push
        EOF

        chmod +x "$out/pre-commit" "$out/pre-push"
      '';
    in
    {
      packages = {
        stitch = stitchCliPkg;
        stitch-unwrapped = stitchCliUnwrapped;
        stitch-mcp = stitchMcpPkg;
        stitch-mcp-unwrapped = stitchMcpUnwrapped;
        default = stitchCliPkg;
      };

      checks = {
        stitch-package = stitchCliPkg;
        stitch-mcp-package = stitchMcpPkg;
        tend-gate = tendGate;
      };

      apps = {
        stitch = {
          type = "app";
          program = "${stitchCliPkg}/bin/stitch";
          meta.description = "Coordinate changes across a discovered multi-repository workspace";
        };
        stitch-mcp = {
          type = "app";
          program = "${stitchMcpPkg}/bin/stitch-mcp";
          meta.description = "Expose Stitch orchestration through an MCP server";
        };
        default = {
          type = "app";
          program = "${stitchCliPkg}/bin/stitch";
          meta.description = "Coordinate changes across a discovered multi-repository workspace";
        };
      };

      devShells.default = pkgs.mkShell {
        name = "phenix-stitch-dev";
        packages = [
          stitchCliPkg
          tendPkg
          tendFix
          tendVerify
          tendPrePush
          pkgs.rust-analyzer
          pkgs.git
          pkgs.nix
          pkgs.jujutsu
        ]
        ++ qualityRuntime;
        shellHook = ''
          if repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"; then
            git -C "$repo_root" config --local core.hooksPath ${gitHooks}
            hooks_status="enabled"
          else
            hooks_status="not in a Git repository"
          fi

          echo "phenix-stitch dev shell"
          echo "  hooks:   $hooks_status"
          echo "  fix:     tend-fix"
          echo "  verify:  tend-verify"
          echo "  prepush: tend-pre-push"
          echo "  stitch:  $(stitch --version 2>/dev/null || echo '?')"
          echo "  tend:    $(tend --version 2>/dev/null || echo '?')"
        '';
      };
    };
}
