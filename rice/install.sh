#!/usr/bin/env bash
# Builds this fork and installs it to ~/.local/bin (ahead of /usr/bin in PATH), and links the
# config (Catppuccin theme, client ID).
set -euo pipefail
REPO="$(cd "$(dirname "$0")/.." && pwd)"

# Same features as Arch's package (defaults + cover).
(cd "$REPO" && cargo build --release --locked --features cover)
install -Dm755 "$REPO/target/release/ncspot" ~/.local/bin/ncspot
rm -f ~/.local/bin/ncspot-next  # side-by-side build from before the ratatui interface was merged

mkdir -p ~/.config/ncspot
ln -sf "$REPO/rice/config.toml" ~/.config/ncspot/config.toml
echo "Installed $(~/.local/bin/ncspot --version)"
