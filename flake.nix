{
  description = "Phenix multi-repo Git coordinator (stitch CLI + MCP)";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    phenix-pins.url = "github:matthis-k/phenix-pins";
    phenix-tend = {
      url = "github:matthis-k/phenix-tend";
      inputs.phenix-pins.follows = "phenix-pins";
    };
    nixpkgs.follows = "phenix-pins/nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      imports = [ ./modules/package.nix ];
      flake.flakeModules.default = import ./modules/flake-module.nix;
    };
}
