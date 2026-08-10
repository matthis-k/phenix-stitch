{
  description = "Dependency-aware ordered execution across Phenix repositories";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    phenix-pins.url = "github:matthis-k/phenix-pins";
    phenix-flake-ci.url = "github:matthis-k/phenix-flake-ci";
    nixpkgs.follows = "phenix-pins/nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      imports = [
        ./modules/package.nix
        ./modules/development.nix
      ];
      flake.flakeModules.default = import ./modules/flake-module.nix;
    };
}
