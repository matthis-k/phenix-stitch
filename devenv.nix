{ lib, ... }:
let
  maintenanceModules = builtins.filter (path: builtins.baseNameOf path == "maintenance.nix") (
    lib.filesystem.listFilesRecursive ./.
  );
in
{
  imports = maintenanceModules;
  enterTest = "";
}
