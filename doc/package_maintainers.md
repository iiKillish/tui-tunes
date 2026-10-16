# Packaging

## Compilation Instructions
ttunes makes use of the standard Cargo build system for everything. To compile a release version,
execute `cargo build --release` in the terminal from the project root. The executable file can be
found at `target/release/ttunes`. For detailed build instructions, have a look at [the developer
documentation](/doc/developers.md).

Additional features can be included by appending them to the build command. A list of all the
available features can be found in the [Cargo.toml](/Cargo.toml) under the `[features]` table. To
activate a feature, include its name like `cargo build --release --features feature1,feature2,...`.
To disable the default features, add `--no-default-features` to the command.

## Other Provided Files
The following is a list of other files that are provided by ttunes. Some of them need to be
generated. Execute `cargo xtask --help` for more information.
- LICENSE
- misc/ttunes.desktop (for Linux systems)
- misc/*.1 (for Linux systems)
- misc/ttunes.bash (bash completions)
- misc/\_ttunes (zsh completions)
- misc/ttunes.fish (fish completions)
- misc/ttunes.elv (elvish completions)
- misc/\_ttunes.ps1 (powershell completions)
