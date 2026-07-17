{ lib, ... }:
{
  perSystem =
    { pkgs, ... }:
    let
      source = lib.cleanSource ../.;
      cargoLock = {
        lockFile = ../Cargo.lock;
      };

      stitchUnwrapped = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch";
        version = "0.1.0";
        src = source;
        inherit cargoLock;
        cargoBuildFlags = "-p stitch-cli";
        nativeBuildInputs = [ pkgs.git ];
      };

      stitchMcpUnwrapped = pkgs.rustPlatform.buildRustPackage {
        pname = "stitch-mcp";
        version = "0.1.0";
        src = source;
        inherit cargoLock;
        cargoBuildFlags = "-p stitch-mcp";
        nativeBuildInputs = [ pkgs.git ];
      };

      runtime = [
        pkgs.git
        pkgs.nix
      ];

      stitch = pkgs.writeShellApplication {
        name = "stitch";
        runtimeInputs = runtime;
        text = ''exec ${stitchUnwrapped}/bin/stitch "$@"'';
      };

      stitchMcp = pkgs.writeShellApplication {
        name = "stitch-mcp";
        runtimeInputs = runtime;
        text = ''exec ${stitchMcpUnwrapped}/bin/stitch-mcp "$@"'';
      };
    in
    {
      packages = {
        inherit stitch;
        stitch-unwrapped = stitchUnwrapped;
        stitch-mcp = stitchMcp;
        stitch-mcp-unwrapped = stitchMcpUnwrapped;
        default = stitch;
      };

      checks = {
        stitch-package = stitch;
        stitch-mcp-package = stitchMcp;
      };

      apps = {
        stitch = {
          type = "app";
          program = "${stitch}/bin/stitch";
        };
        stitch-mcp = {
          type = "app";
          program = "${stitchMcp}/bin/stitch-mcp";
        };
        default = {
          type = "app";
          program = "${stitch}/bin/stitch";
        };
      };

      devShells.default = pkgs.mkShell {
        name = "phenix-stitch-dev";
        packages = [
          stitch
          stitchMcp
          pkgs.devenv
          pkgs.cargo
          pkgs.rustc
          pkgs.rustfmt
          pkgs.clippy
          pkgs.rust-analyzer
          pkgs.git
          pkgs.nix
        ];
        shellHook = ''
          echo "phenix-stitch development shell"
          echo "  maintenance: devenv test"
          echo "  fixes:       devenv tasks run maintenance:fix"
          echo "  stitch:      $(stitch --version 2>/dev/null || echo '?')"
        '';
      };
    };
}
