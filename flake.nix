{
  description = "A basic rust project";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    supportedSystems = [
      "x86_64-linux"
      "aarch64-linux"
      "x86_64-darwin"
      "aarch64-darwin"
    ];

    eachSystem = nixpkgs.lib.genAttrs supportedSystems (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        nativeBuildInputs = with pkgs; [
          cargo
          rustc
          rustfmt
          rust-analyzer
          clippy
        ];
        buildInputs = [];
      in {
        devShell = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;
          shellHook = ''
            echo "Rust toolchain: $(rustc --version)"
            echo "Rust analyzer: $(rust-analyzer --version)"
            echo "Clippy: $(clippy-driver --version)"
          '';
        };
      }
    );
  in {
    devShells =
      nixpkgs.lib.mapAttrs (system: systemAttrs: {
        default = systemAttrs.devShell;
      })
      eachSystem;
  };
}
