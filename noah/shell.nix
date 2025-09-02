{
  pkgs ? import <nixpkgs> { },
}:
with pkgs;
mkShell {
  strictDeps = true;

  nativeBuildInputs = [
    cargo
    rustc

    rust-analyzer-unwrapped
    (rustfmt.override { asNightly = true; })
    clippy
    nix-output-monitor
    taplo
    yaml-language-server
    lldb
  ];

  buildInputs = lib.optionals stdenv.isDarwin [
    libiconv
  ];

  env = {
    NH_LOG = "nh=trace";
    RUST_SRC_PATH = "${rustPlatform.rustLibSrc}";
  };
}
