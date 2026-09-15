{ pkgs, ... }:

{
  languages.rust.enable = true;

  packages = [
    pkgs.cargo-nextest
    pkgs.openssl
    pkgs.pkg-config
  ];

  env.PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

  dotenv = {
    enable = true;
    filename = ".env";
  };

  enterShell = ''
    if [ -f "$DEVENV_ROOT/.env.local" ]; then
      set -a
      . "$DEVENV_ROOT/.env.local"
      set +a
    fi
  '';

  enterTest = ''
    cargo build --locked
    cargo nextest run --locked
  '';
}
