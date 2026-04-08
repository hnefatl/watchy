{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    esp-rs-nix = {
      url = "github:leighleighleigh/esp-rs-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      esp-rs-nix,
    }:
    let
      system = "x86_64-linux";
    in
    {
      devShells.${system}.default = esp-rs-nix.devShells.${system}.default;
    };
}
