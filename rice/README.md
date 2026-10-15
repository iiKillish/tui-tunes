# ncspot (rice fork)

Fork of [ncspot](https://github.com/hrkfdn/ncspot) 1.4.0 (`upstream` remote) used as the music
player. Changes on top, one commit each:

- `client_id` config option: Web API requests use your own Spotify app instead of the shared,
  rate-limited client ID
- no UI freezes on long rate-limit waits
- MPRIS capability changes are announced (KDE's media controls see it as controllable)
- shuffle reorders the queue itself, so the queue view shows the real play order
- Spotify-style player bar replacing the status bar: cover art, clickable controls drawn as
  images (Kitty graphics), progress and volume
- queue panel on the right: now playing, then "Next from: <album/playlist>"
- covers drawn without moving the cursor or deleting other images (fixes stale drawing)

`rice/config.toml` is the config (Catppuccin theme, client ID); `rice/install.sh` builds with
Arch's features, installs to `~/.local/bin/ncspot` and links the config.
