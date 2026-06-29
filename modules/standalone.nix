{ inputs, lib, ... }: {
  perSystem =
    { config, pkgs, system, ... }:
    let
      filteredSrc = lib.cleanSource ../.;

      stitchCliPkg = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch";
        version = "0.1.0";
        src = filteredSrc;
        cargoLock.lockFile = ../Cargo.lock;
        cargoBuildFlags = "-p stitch-cli";
        nativeBuildInputs = [ pkgs.git ];
      };

      stitchMcpPkg = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch-mcp";
        version = "0.1.0";
        src = filteredSrc;
        cargoLock.lockFile = ../Cargo.lock;
        cargoBuildFlags = "-p stitch-mcp";
        nativeBuildInputs = [ pkgs.git ];
      };

      # Reuse vendored crate dependencies from any buildRustPackage.
      cargoDeps = stitchCliPkg.cargoDeps or (throw "cargoDeps not found");

      mkCargoCheck =
        name: description: cargoArgs: extraNativeBuildInputs:
        pkgs.runCommand name
          {
            nativeBuildInputs = extraNativeBuildInputs ++ [ pkgs.stdenv.cc ];
            inherit cargoDeps;
            src = filteredSrc;
          }
          ''
            export HOME=$TMPDIR/home
            mkdir -p $HOME
            export CARGO_HOME=$TMPDIR/cargo
            export CARGO_TARGET_DIR=$TMPDIR/target
            mkdir -p $CARGO_HOME $CARGO_TARGET_DIR

            cp -rT $src source
            chmod -R u+w source
            cd source

            # Point cargo at the vendored dependencies
            mkdir -p .cargo
            cat > .cargo/config.toml <<EOF
            [source.crates-io]
            replace-with = "vendored-sources"

            [source.vendored-sources]
            directory = "${cargoDeps}"
            EOF

            ${cargoArgs}

            touch $out
          '';
    in
    {
      packages = {
        inherit
          stitchCliPkg
          stitchMcpPkg
          ;
        stitch = stitchCliPkg;
        stitch-mcp = stitchMcpPkg;
        default = stitchCliPkg;
      };

      checks = {
        cargo-check =
          mkCargoCheck "phenix-stitch-cargo-check" "cargo check --workspace --all-targets"
            "cargo check --workspace --all-targets"
            [
              pkgs.cargo
              pkgs.rustc
            ];

        cargo-test =
          mkCargoCheck "phenix-stitch-cargo-test" "cargo test --workspace" "cargo test --workspace"
            [
              pkgs.cargo
              pkgs.rustc
              pkgs.git
            ];

        cargo-fmt =
          mkCargoCheck "phenix-stitch-cargo-fmt" "cargo fmt --all --check" "cargo fmt --all --check"
            [
              pkgs.cargo
              pkgs.rustfmt
            ];

        cargo-clippy =
          mkCargoCheck "phenix-stitch-cargo-clippy"
            "cargo clippy --quiet --workspace --all-targets -- -D warnings"
            "cargo clippy --quiet --workspace --all-targets -- -D warnings"
            [
              pkgs.cargo
              pkgs.clippy
              pkgs.rustc
            ];

        tend-gate =
          pkgs.runCommand "phenix-stitch-tend-gate"
            {
              nativeBuildInputs = [
                inputs.phenix-tend.packages.${system}.tend
                stitchCliPkg
                pkgs.git
                pkgs.cargo
                pkgs.rustc
                pkgs.rustfmt
                pkgs.clippy
                pkgs.nixfmt
                pkgs.statix
                pkgs.deadnix
                pkgs.stdenv.cc
              ];
              inherit cargoDeps;
              src = filteredSrc;
            }
            ''
              export HOME=$TMPDIR/home
              mkdir -p $HOME
              export CARGO_HOME=$TMPDIR/cargo
              export CARGO_TARGET_DIR=$TMPDIR/target
              mkdir -p $CARGO_HOME $CARGO_TARGET_DIR

              cp -rT $src source
              chmod -R u+w source
              cd source

              # Point cargo at the vendored dependencies
              mkdir -p .cargo
              cat > .cargo/config.toml <<EOF
              [source.crates-io]
              replace-with = "vendored-sources"

              [source.vendored-sources]
              directory = "${cargoDeps}"
              EOF

              # git is needed by stitch for changed-file detection
              git init && git add -A

              tend run --mode full --phase verify --profile nix-check

              touch $out
            '';
      };

      apps = {
        stitch = {
          type = "app";
          program = "${stitchCliPkg}/bin/stitch";
        };
        stitch-mcp = {
          type = "app";
          program = "${stitchMcpPkg}/bin/stitch-mcp";
        };
        default = {
          type = "app";
          program = "${stitchCliPkg}/bin/stitch";
        };
      };

      devShells.default = pkgs.mkShell {
        name = "phenix-stitch-dev";
        packages = [
          pkgs.cargo
          pkgs.rustc
          pkgs.rustfmt
          pkgs.clippy
          pkgs.rust-analyzer
          pkgs.git
          pkgs.nix
          stitchCliPkg
        ];
        shellHook = ''
          echo "phenix-stitch dev shell"
          echo "  cargo: $(cargo --version 2>/dev/null || echo '?')"
          echo "  rustc: $(rustc --version 2>/dev/null || echo '?')"
          echo "  stitch: $(stitch --version 2>/dev/null || echo '?')"
        '';
      };
    };
}
