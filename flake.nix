{
  description = "TerrainTile - headless terrain pipeline with web UI and 3D viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      terraintile = pkgs.rustPlatform.buildRustPackage {
        pname = "terraintile";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
      };
    in
    {
      packages.${system}.default = terraintile;
      apps.${system}.default = {
        type = "app";
        program = "${terraintile}/bin/terraintile";
      };
      devShells.${system}.default = pkgs.mkShell {
        inputsFrom = [ terraintile ];
        packages = with pkgs; [ cargo rustc rustfmt clippy ];
      };
    };
}
