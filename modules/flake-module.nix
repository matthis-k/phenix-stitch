{ inputs, ... }: {
  perSystem = { system, ... }: {
    phenixWrapped = {
      stitch = inputs.phenix-stitch.packages.${system}.stitch;
    };
  };
}
