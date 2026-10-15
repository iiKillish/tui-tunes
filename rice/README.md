# ncspot (rice fork)

Fork of [ncspot](https://github.com/hrkfdn/ncspot) 1.5.0 (`upstream` remote) used as the music
player. Changes on top, one commit each:

- `client_id` config option: Web API requests use your own Spotify app instead of the shared,
  rate-limited client ID
- no UI freezes on long rate-limit waits
- MPRIS capability changes are announced (KDE's media controls see it as controllable)
- shuffle reorders the queue itself, so the queue view shows the real play order
- Spotify-style player bar replacing the status bar: cover art, clickable controls drawn as
  images (Kitty graphics), progress and volume
- side panel on the right with two tabs: the queue (now playing, then "Next from:
  <album/playlist>") and time-synced lyrics from LRCLIB; `Tab` focuses it for the keyboard,
  `Shift+L` switches to the lyrics
- home screen (`F4`, the start screen in `rice/config.toml`): cards with covers for jump back
  in, recently played, your top tracks and top artists
- album, playlist and artist pages start with a Spotify-style header: big cover, name, artist or
  owner, year, number of songs and length
- like button (heart) after the title in the player bar, also `Shift+H` or `:like`
- covers drawn without moving the cursor or deleting other images (fixes stale drawing)

`rice/config.toml` is the config (Catppuccin theme, client ID); `rice/install.sh` builds with
Arch's features, installs to `~/.local/bin/ncspot` and links the config.
