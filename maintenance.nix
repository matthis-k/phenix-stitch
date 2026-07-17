{ pkgs, ... }:
let
  root = ''repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"; cd "$repo_root"'';
  nixSources = "find . -type f -name '*.nix' -not -path './.git/*' -not -path './.devenv/*' -not -path './target/*'";
in
{
  scripts = {
    "maintenance-check-rust-format" = {
      packages = [
        pkgs.cargo
        pkgs.rustfmt
        pkgs.git
      ];
      exec = "${root}; cargo fmt --all --check";
    };
    "maintenance-check-rust-compile" = {
      packages = [
        pkgs.cargo
        pkgs.rustc
        pkgs.git
      ];
      exec = "${root}; cargo check --workspace --all-targets";
    };
    "maintenance-check-rust-clippy" = {
      packages = [
        pkgs.cargo
        pkgs.rustc
        pkgs.clippy
        pkgs.git
      ];
      exec = "${root}; cargo clippy --workspace --all-targets -- -D warnings";
    };
    "maintenance-check-rust-tests" = {
      packages = [
        pkgs.cargo
        pkgs.rustc
        pkgs.git
      ];
      exec = "${root}; cargo test --workspace";
    };
    "maintenance-check-nix-format" = {
      packages = [
        pkgs.findutils
        pkgs.git
        pkgs.nixfmt
      ];
      exec = "${root}; ${nixSources} -exec nixfmt --check {} +";
    };
    "maintenance-check-statix" = {
      packages = [
        pkgs.git
        pkgs.statix
      ];
      exec = "${root}; statix check --ignore '.git/**' --ignore 'target/**' ";
    };
    "maintenance-check-deadnix" = {
      packages = [
        pkgs.deadnix
        pkgs.git
      ];
      exec = "${root}; deadnix --fail --no-lambda-arg --no-lambda-pattern-names";
    };
    "maintenance-check-flake" = {
      packages = [
        pkgs.git
        pkgs.nix
      ];
      exec = "${root}; nix flake check --print-build-logs --keep-going";
    };
    "maintenance-fix-statix" = {
      packages = [
        pkgs.git
        pkgs.statix
      ];
      exec = "${root}; statix fix";
    };
    "maintenance-fix-format" = {
      packages = [
        pkgs.cargo
        pkgs.findutils
        pkgs.git
        pkgs.nixfmt
        pkgs.rustfmt
      ];
      exec = ''
        ${root}
        cargo fmt --all
        ${nixSources} -exec nixfmt {} +
      '';
    };
    "maintenance-fix-deadnix" = {
      packages = [
        pkgs.deadnix
        pkgs.git
      ];
      exec = "${root}; deadnix --edit --no-lambda-arg --no-lambda-pattern-names";
    };
  };

  tasks = {
    "maintenance:rust-format".exec = "maintenance-check-rust-format";
    "maintenance:rust-compile".exec = "maintenance-check-rust-compile";
    "maintenance:rust-clippy".exec = "maintenance-check-rust-clippy";
    "maintenance:rust-tests".exec = "maintenance-check-rust-tests";
    "maintenance:nix-format".exec = "maintenance-check-nix-format";
    "maintenance:statix".exec = "maintenance-check-statix";
    "maintenance:deadnix".exec = "maintenance-check-deadnix";
    "maintenance:flake".exec = "maintenance-check-flake";

    "maintenance:check" = {
      exec = "true";
      after = [
        "maintenance:rust-format"
        "maintenance:rust-compile"
        "maintenance:rust-clippy"
        "maintenance:rust-tests"
        "maintenance:nix-format"
        "maintenance:statix"
        "maintenance:deadnix"
        "maintenance:flake"
      ];
      before = [ "devenv:enterTest" ];
    };

    "maintenance:fix:statix".exec = "maintenance-fix-statix";
    "maintenance:fix:deadnix" = {
      exec = "maintenance-fix-deadnix";
      after = [ "maintenance:fix:statix" ];
    };
    "maintenance:fix:format" = {
      exec = "maintenance-fix-format";
      after = [ "maintenance:fix:deadnix" ];
    };
    "maintenance:fix" = {
      exec = "true";
      after = [ "maintenance:fix:format" ];
    };
  };
}
