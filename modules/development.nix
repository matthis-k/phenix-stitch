{ inputs, ... }:
{
  perSystem =
    { config, pkgs, ... }:
    let
      maintenanceLib = inputs.phenix-flake-ci.lib;
      repositoryRoot = ''
        repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
        cd "$repo_root"
      '';

      sourceCi = {
        enable = true;
        stage = "source";
        name = "Source";
        timeoutMinutes = 30;
      };
      productCi = {
        enable = true;
        stage = "product";
        name = "Product";
        timeoutMinutes = 45;
        needs = [ "source" ];
      };

      maintenance = maintenanceLib.mkMaintenance {
        name = "maintenance";
        description = "Phenix Stitch maintenance";
        ci.github = {
          enable = true;
          outputName = "phenix-maintenance";
        };
        gitHooks = {
          enable = true;
          preCommit = [ "fix" ];
        };

        commands = {
          all = {
            description = "Run the complete validation graph";
            exec = ''
              "$0" check
              "$0" test
            '';
          };

          check = {
            description = "Run source and compiler validation";
            order = [
              "nix-format"
              "rust-format"
              "statix"
              "deadnix"
              "actionlint"
              "clippy"
              "flake-eval"
              "workflow-sync"
            ];
            commands = {
              nix-format = {
                description = "Nix formatting";
                ci = sourceCi // {
                  stepName = "Nix formatting";
                };
                runtimeInputs = pkgs: [
                  pkgs.findutils
                  pkgs.git
                  pkgs.nixfmt
                ];
                exec = ''
                  ${repositoryRoot}
                  find . -type f -name '*.nix' \
                    -not -path './.git/*' \
                    -print0 |
                    xargs -0 -r nixfmt --check
                '';
              };

              rust-format = {
                description = "Rust formatting";
                ci = sourceCi // {
                  stepName = "Rust formatting";
                };
                runtimeInputs = pkgs: [
                  pkgs.cargo
                  pkgs.git
                  pkgs.rustfmt
                ];
                exec = ''
                  ${repositoryRoot}
                  cargo fmt --all -- --check
                '';
              };

              statix = {
                description = "Nix static analysis";
                ci = sourceCi // {
                  stepName = "Statix";
                };
                runtimeInputs = pkgs: [
                  pkgs.git
                  pkgs.statix
                ];
                exec = ''
                  ${repositoryRoot}
                  statix check --ignore '.git/**'
                '';
              };

              deadnix = {
                description = "Unused Nix code";
                ci = sourceCi // {
                  stepName = "Deadnix";
                };
                runtimeInputs = pkgs: [
                  pkgs.deadnix
                  pkgs.git
                ];
                exec = ''
                  ${repositoryRoot}
                  deadnix --fail --no-lambda-arg --no-lambda-pattern-names
                '';
              };

              actionlint = {
                description = "GitHub Actions syntax";
                ci = sourceCi // {
                  stepName = "Actionlint";
                };
                runtimeInputs = pkgs: [
                  pkgs.actionlint
                  pkgs.findutils
                  pkgs.git
                ];
                exec = ''
                  ${repositoryRoot}
                  find .github/workflows -type f \
                    \( -name '*.yml' -o -name '*.yaml' \) -print0 |
                    xargs -0 -r actionlint
                '';
              };

              clippy = {
                description = "Rust compiler linting";
                ci = sourceCi // {
                  stepName = "Clippy";
                };
                runtimeInputs = pkgs: [
                  pkgs.cargo
                  pkgs.clippy
                  pkgs.git
                  pkgs.rustc
                ];
                exec = ''
                  ${repositoryRoot}
                  cargo clippy --workspace --all-targets --locked -- -D warnings
                '';
              };

              flake-eval = {
                description = "Flake output evaluation";
                ci = sourceCi // {
                  stepName = "Flake evaluation";
                };
                runtimeInputs = pkgs: [
                  pkgs.git
                  pkgs.nix
                ];
                exec = ''
                  ${repositoryRoot}
                  nix flake check --no-build --print-build-logs
                '';
              };

              workflow-sync = {
                description = "Committed workflow matches the maintenance declaration";
                ci = sourceCi // {
                  stepName = "Generated workflow";
                };
                runtimeInputs = pkgs: [
                  pkgs.diffutils
                  pkgs.git
                  pkgs.nix
                ];
                exec = ''
                  ${repositoryRoot}
                  system="$(nix eval --impure --raw --expr builtins.currentSystem)"
                  generated="$(mktemp)"
                  trap 'rm -f "$generated"' EXIT
                  nix eval --raw \
                    ".#packages.$system.phenix-maintenance.phenixMaintenance.ci.github.workflow" \
                    > "$generated"
                  diff -u .github/workflows/ci.yml "$generated"
                '';
              };
            };
          };

          test = {
            description = "Run functional Stitch tests";
            order = [
              "rust"
              "cli"
            ];
            commands = {
              rust = {
                description = "Execute the Rust workspace test suite";
                ci = productCi // {
                  stepName = "Rust tests";
                };
                runtimeInputs = pkgs: [
                  pkgs.cargo
                  pkgs.git
                  pkgs.rustc
                ];
                exec = ''
                  ${repositoryRoot}
                  cargo test --workspace --locked
                '';
              };

              cli = {
                description = "Exercise the packaged CLI entry points";
                ci = productCi // {
                  stepName = "CLI smoke tests";
                };
                runtimeInputs = pkgs: [
                  pkgs.git
                  pkgs.nix
                ];
                exec = ''
                  ${repositoryRoot}
                  nix run .#stitch -- --version
                  nix run .#stitch-mcp -- --help >/dev/null
                '';
              };
            };
          };

          fix = {
            description = "Apply deterministic source normalization";
            runtimeInputs = pkgs: [
              pkgs.cargo
              pkgs.deadnix
              pkgs.findutils
              pkgs.git
              pkgs.nixfmt
              pkgs.rustfmt
              pkgs.statix
            ];
            exec = ''
              ${repositoryRoot}
              statix fix
              deadnix --edit --no-lambda-arg --no-lambda-pattern-names
              find . -type f -name '*.nix' \
                -not -path './.git/*' \
                -print0 |
                xargs -0 -r nixfmt
              cargo fmt --all
            '';
          };
        };
      };

      maintenancePackage = maintenanceLib.mkMaintenancePackage {
        inherit pkgs maintenance;
      };
    in
    {
      packages.phenix-maintenance = maintenancePackage.package;
      apps.phenix-maintenance = maintenancePackage.app;

      devShells.default = pkgs.mkShell {
        name = "phenix-stitch-dev";
        packages = [
          config.packages.stitch
          config.packages.stitch-mcp
          pkgs.cargo
          pkgs.clippy
          pkgs.git
          pkgs.nix
          pkgs.rust-analyzer
          pkgs.rustc
          pkgs.rustfmt
          maintenancePackage.package
        ];
        shellHook = ''
          ${maintenancePackage.shellHook}

          echo "phenix-stitch development shell"
          echo "  all:    maintenance all"
          echo "  check:  maintenance check"
          echo "  test:   maintenance test"
          echo "  fixes:  maintenance fix"
        '';
      };
    };
}
