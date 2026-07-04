{ inputs, lib, ... }: {
  perSystem =
    {
      config,
      pkgs,
      system,
      ...
    }:
    let
      filteredSrc = lib.cleanSource ../.;

      rustToolchain = [
        pkgs.cargo
        pkgs.rustc
        # These provide cargo-fmt and cargo-clippy subcommands for Tend.
        pkgs.rustfmt
        pkgs.clippy
      ];

      stitchCliPkg = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch";
        version = "0.1.0";
        src = filteredSrc;
        cargoLock = {
          lockFile = ../Cargo.lock;
          outputHashes = {
            "phenix-mcp-core-0.1.0" = "sha256-6XxX63SIZ8RgQRCvhHx1M5p1wkUAnCsDJawljTCRXIo=";
            "tend-0.1.0" = "sha256-DotfuPsjXINPAA+YGxmu1B2JytHrbLi8SPIem14Y6pQ=";
          };
        };
        cargoBuildFlags = "-p stitch-cli";
        nativeBuildInputs = [ pkgs.git ];
      };

      stitchMcpPkg = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch-mcp";
        version = "0.1.0";
        src = filteredSrc;
        cargoLock = {
          lockFile = ../Cargo.lock;
          outputHashes = {
            "phenix-mcp-core-0.1.0" = "sha256-6XxX63SIZ8RgQRCvhHx1M5p1wkUAnCsDJawljTCRXIo=";
            "tend-0.1.0" = "sha256-DotfuPsjXINPAA+YGxmu1B2JytHrbLi8SPIem14Y6pQ=";
          };
        };
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

            # Reuse the buildRustPackage-generated Cargo config so git sources
            # are replaced with the vendored source directory too.
            cp -r ${cargoDeps}/.cargo .cargo
            ln -s ${cargoDeps} cargo-vendor-dir

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
            rustToolchain;

        cargo-test =
          mkCargoCheck "phenix-stitch-cargo-test" "cargo test --workspace" "cargo test --workspace"
            [
              pkgs.cargo
              pkgs.rustc
              pkgs.git
            ];

        cargo-fmt =
          mkCargoCheck "phenix-stitch-cargo-fmt" "cargo fmt --all --check" "cargo fmt --all --check"
            rustToolchain;

        cargo-clippy =
          mkCargoCheck "phenix-stitch-cargo-clippy"
            "cargo clippy --quiet --workspace --all-targets -- -D warnings"
            "cargo clippy --quiet --workspace --all-targets -- -D warnings"
            rustToolchain;

        local-gate =
          pkgs.runCommand "phenix-stitch-local-gate"
            {
              nativeBuildInputs = [
                stitchCliPkg
                pkgs.git
                pkgs.nixfmt
                pkgs.statix
                pkgs.deadnix
                pkgs.stdenv.cc
              ]
              ++ rustToolchain;
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

              # Reuse the buildRustPackage-generated Cargo config so git sources
              # are replaced with the vendored source directory too.
              cp -r ${cargoDeps}/.cargo .cargo
              ln -s ${cargoDeps} cargo-vendor-dir

              # git is needed by stitch for changed-file detection
              git init && git add -A

              cargo fmt --all --check
              cargo check --workspace --all-targets
              cargo clippy --quiet --workspace --all-targets -- -D warnings
              cargo test --workspace
              find modules -name '*.nix' -print0 | xargs -0 -r nixfmt --check
              find . -maxdepth 1 -name '*.nix' -print0 | xargs -0 -r nixfmt --check
              statix check
              deadnix --fail --no-lambda-arg --no-lambda-pattern-names

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
          pkgs.rust-analyzer
          pkgs.git
          pkgs.nix
          pkgs.jujutsu
          stitchCliPkg
        ]
        ++ rustToolchain;
        shellHook = ''
          echo "phenix-stitch dev shell"
          echo "  cargo: $(cargo --version 2>/dev/null || echo '?')"
          echo "  rustc: $(rustc --version 2>/dev/null || echo '?')"
          echo "  stitch: $(stitch --version 2>/dev/null || echo '?')"
          echo "  jj: $(jj --version 2>/dev/null || echo 'not installed')"
        '';
      };
    };
}
