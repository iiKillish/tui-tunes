# ncspot (rice fork)

Fork of [ncspot](https://github.com/hrkfdn/ncspot) 1.5.0 (`upstream` remote) used as the music
player, with a new interface written with [ratatui](https://ratatui.rs) (the design is in
[DESIGN.md](DESIGN.md)). The old cursive interface, with the same playback fixes, is on the
`cursive` branch.

- **Interface**: a Spotify-style now-playing hero (big cover, track, controls, progress, volume
  with a draggable knob), screens for Home, Search, Library, Up next and Lyrics, cover grids,
  album/playlist/artist pages with a tinted header and Play / Shuffle / Like buttons, mouse
  support everywhere. Covers and the control icons use Kitty graphics (Ghostty, Kitty, WezTerm).
- **Lyrics**: time-synced lyrics from LRCLIB.
- **Like** the current song with the heart, `Shift+H` or `:like`.
- `client_id` config option: Web API requests use your own Spotify app instead of the shared,
  rate-limited client ID; no UI freezes on long rate-limit waits.
- MPRIS capability changes are announced (KDE's media controls see it as controllable).
- Shuffle reorders the queue itself, so Up next shows the real play order.
- `volume_curve` option; cubic by default, like system mixers.

`rice/config.toml` is the config (Catppuccin theme from the theme project, client ID);
`rice/install.sh` builds with Arch's features (plus covers), installs to `~/.local/bin/ncspot` and
links the config. Run it after pulling.
