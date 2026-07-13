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

      cargoLock = {
        lockFile = ../Cargo.lock;
        outputHashes = {
          "phenix-mcp-core-0.1.0" = "sha256-6XxX63SIZ8RgQRCvhHx1M5p1wkUAnCsDJawljTCRXIo=";
        };
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

      mkCargoCheck =
        name: cargoArgs: extraNativeBuildInputs:
        pkgs.runCommand name
          {
            nativeBuildInputs = extraNativeBuildInputs ++ [ pkgs.stdenv.cc ];
            inherit cargoDeps;
            src = source;
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

            cp -r ${cargoDeps}/.cargo .cargo
            ln -s ${cargoDeps} cargo-vendor-dir

            ${cargoArgs}

            touch $out
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
        cargo-check =
          mkCargoCheck "phenix-stitch-cargo-check" "cargo check --workspace --all-targets"
            rustToolchain;

        cargo-test = mkCargoCheck "phenix-stitch-cargo-test" "cargo test --workspace" [
          pkgs.cargo
          pkgs.rustc
          pkgs.git
        ];

        cargo-fmt =
          mkCargoCheck "phenix-stitch-cargo-fmt" "cargo fmt --all --check" rustToolchain;

        cargo-clippy =
          mkCargoCheck "phenix-stitch-cargo-clippy"
            "cargo clippy --quiet --workspace --all-targets -- -D warnings"
            rustToolchain;

        tend-config = pkgs.runCommand "phenix-stitch-tend-config" {
          nativeBuildInputs = [ tendPkg ];
          src = source;
        } ''
          cp -rT $src source
          chmod -R u+w source
          tend --root source validate
          touch $out
        '';

        local-gate =
          pkgs.runCommand "phenix-stitch-local-gate"
            {
              nativeBuildInputs = [
                stitchCliPkg
                tendPkg
                pkgs.git
                pkgs.nixfmt
                pkgs.statix
                pkgs.deadnix
                pkgs.stdenv.cc
              ]
              ++ rustToolchain;
              inherit cargoDeps;
              src = source;
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

              cp -r ${cargoDeps}/.cargo .cargo
              ln -s ${cargoDeps} cargo-vendor-dir

              git init --quiet
              git add -A

              tend --root . validate
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
          stitchCliPkg
          tendPkg
          pkgs.rust-analyzer
          pkgs.git
          pkgs.nix
          pkgs.nixfmt
          pkgs.statix
          pkgs.deadnix
          pkgs.jujutsu
        ]
        ++ rustToolchain;
        shellHook = ''
          echo "phenix-stitch dev shell"
          echo "  cargo:  $(cargo --version 2>/dev/null || echo '?')"
          echo "  rustc:  $(rustc --version 2>/dev/null || echo '?')"
          echo "  stitch: $(stitch --version 2>/dev/null || echo '?')"
          echo "  tend:   $(tend --version 2>/dev/null || echo '?')"
          echo "  jj:     $(jj --version 2>/dev/null || echo '?')"
        '';
      };
    };
}
