{
  description = "zoho-mail-sync: one-way Maildir mirror of a Zoho Mail account";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        package = pkgs.rustPlatform.buildRustPackage {
          pname = "zoho-mail-sync";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ];
          meta = {
            description = "One-way Maildir mirror of a Zoho Mail account";
            homepage = "https://github.com/vijayvar/zoho-mail-sync";
            license = pkgs.lib.licenses.mit;
            mainProgram = "zoho-mail-sync";
            platforms = pkgs.lib.platforms.unix;
          };
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rust-analyzer
            pkgs.clippy
            pkgs.rustfmt
            pkgs.pkg-config
            pkgs.openssl
          ];
          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
        };

        packages.default = package;

        apps.default = {
          type = "app";
          program = "${package}/bin/zoho-mail-sync";
        };
      });
}
