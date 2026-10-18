# ttunes

A Spotify player for the terminal, written in Rust with
[librespot](https://github.com/librespot-org/librespot) and [ratatui](https://ratatui.rs). It
needs a Spotify Premium account.

- A now-playing view with a big cover, the track, progress, controls and a volume slider, beside
  or above the _Home_, _Search_, _Library_, _Up next_ and _Lyrics_ screens
- Album, playlist and artist pages, and cover grids for the library and search results
- Time-synced lyrics from [LRCLIB](https://lrclib.net)
- Autoplay: similar songs keep playing when the queue runs out
- Album covers in terminals that can show images: Kitty, Ghostty, WezTerm, Konsole, foot,
  Windows Terminal (with WSL), iTerm2 and others with sixel
- Uses your terminal's colors, so it matches the theme you already have
- Listening stats: time listened, streaks, your top songs and artists
- A settings page for library tabs, keys and more; hold Ctrl+Shift to see the keys on screen
- Vim-style keys, mouse support, MPRIS and an IPC socket for remote control

## Install

```sh
cargo install --locked ttunes
```

Building needs a [Rust installation](https://www.rust-lang.org/tools/install), a C compiler,
`pkgconf` and the OpenSSL and PulseAudio development headers (ALSA with the `alsa_backend`
feature). MPRIS and notifications use D-Bus. On Debian and Ubuntu:

```sh
sudo apt install build-essential pkg-config libssl-dev libpulse-dev
```

The same command works on macOS, where ttunes plays through CoreAudio and needs no PulseAudio.

Or download a ready-made build for macOS, Linux or Windows from the
[releases](https://github.com/iiKillish/tui-tunes/releases/latest).

Other distributions and building from a checkout are covered in
[the developer docs](doc/developers.md).

## First run

ttunes needs a Spotify app of your own for Spotify's Web API. The first time it starts, it asks
for the app's client ID:

1. Create an app at https://developer.spotify.com/dashboard.
2. Add the redirect URI `http://127.0.0.1:8989/login`.
3. Choose the Web API.
4. Paste the app's client ID into ttunes and press <kbd>Enter</kbd>.

The ID is saved in `~/.config/ttunes/client_id`, so you're only asked once. Then ttunes opens two
Spotify logins in your browser, one for playback and one for your app.

## Keys and configuration

<kbd>?</kbd> shows the keys. <kbd>Tab</kbd> and <kbd>F1</kbd>–<kbd>F5</kbd> switch screens,
<kbd>Enter</kbd> plays, <kbd>Shift</kbd>+<kbd>P</kbd> pauses, <kbd>:</kbd> opens the command line
and <kbd>q</kbd> quits.

Everything works without a configuration file. The optional `~/.config/ttunes/config.toml` changes
keys, colors and behavior; the full key list and every option are in
[the user docs](doc/users.md).

## Credits

Based on [ncspot](https://github.com/hrkfdn/ncspot) by Henrik Friedrichsen and contributors
(BSD 2-Clause). See [LICENSE](LICENSE) and [CHANGELOG.md](CHANGELOG.md).
