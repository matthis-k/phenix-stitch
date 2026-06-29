{
  pkgs ? import <nixpkgs> { },
}:

pkgs.mkShell {
  packages = with pkgs; [
    nix
    nixfmt
    statix
    deadnix
    cargo
    rustc
    # These provide cargo-fmt and cargo-clippy subcommands for Tend.
    rustfmt
    clippy
  ];
}
