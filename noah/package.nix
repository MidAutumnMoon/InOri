{
  lib,
  rustPlatform,
  makeBinaryWrapper,
  installShellFiles,
  versionCheckHook,
  sudo,
  use-nom ? true,
  nix-output-monitor ? null,
  rev ? "dirty",
}:
assert use-nom -> nix-output-monitor != null;
let
  runtimeDeps = lib.optionals use-nom [ nix-output-monitor ];
  cargoToml = lib.importTOML ./Cargo.toml;
in
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "nh";
  version = "${cargoToml.workspace.package.version}-${rev}";

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.intersection (lib.fileset.fromSource (lib.sources.cleanSource ./.)) (
      lib.fileset.unions [
        ./.cargo
        ./.config
        ./crates
        ./Cargo.toml
        ./Cargo.lock
      ]
    );
  };

  strictDeps = true;
  nativeBuildInputs = [
    makeBinaryWrapper
  ];

  cargoLock.lockFile = ./Cargo.lock;

  postFixup = ''
    wrapProgram $out/bin/nh \
      --prefix PATH : ${lib.makeBinPath runtimeDeps}
  '';

  nativeInstallCheckInputs = [ versionCheckHook ];
  doInstallCheck = false; # FIXME: --version includes 'dirty' and the hook doesn't let us change the assertion
  versionCheckProgram = "${placeholder "out"}/bin/${finalAttrs.meta.mainProgram}";
  versionCheckProgramArg = "--version";

  nativeCheckInputs = [ sudo ];
  checkFlags = [
    # These do not work in Nix's sandbox
    "--skip"
    "test_get_build_image_variants_expression"
    "--skip"
    "test_get_build_image_variants_file"
    "--skip"
    "test_get_build_image_variants_flake"
  ];

  # Besides the install check, we have a bunch of tests to run. Nextest is
  # the fastest way of running those since it's significantly faster than
  # `cargo test`, and has a nicer UI with CI-friendly characteristics.
  useNextest = true;
  cargoTestFlags = [ "--workspace" ];

  env.NH_REV = rev;

  meta = {
    description = "Yet another nix cli helper";
    homepage = "https://github.com/nix-community/nh";
    license = lib.licenses.eupl12;
    mainProgram = "nh";
    maintainers = with lib.maintainers; [
      drupol
      faukah
      NotAShelf
      viperML
    ];
  };
})
