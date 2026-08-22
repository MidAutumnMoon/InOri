pub mod args;
pub mod generations;
#[expect(
    clippy::module_inception,
    reason = "the module mirrors the `nixos` subcommand name"
)]
pub mod nixos;
