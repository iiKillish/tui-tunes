# User Documentation

## Installation Instructions

### From [crates.io](https://crates.io/crates/ttunes)

```sh
# --locked builds with the exact dependency versions ttunes was tested with
cargo install --locked ttunes
```

Album covers (the `cover` feature) are included. To build without them, use
`cargo install --locked ttunes --no-default-features --features pulseaudio_backend,mpris,notify,share_clipboard`.

Building needs a [Rust installation](https://www.rust-lang.org/tools/install), `pkgconf`, and the
development headers for PulseAudio (or ALSA, if built with the ALSA backend), OpenSSL and dbus.
To build from a checkout instead, see [Compiling](/doc/developers.md#compiling).

### macOS
macOS has no PulseAudio, so the default build fails to link (`library ':libpulse-simple.0.dylib'
not found`). Build with the Rodio backend, which plays through CoreAudio, instead:

```sh
cargo install --locked ttunes --no-default-features --features rodio_backend,cover,share_clipboard,notify
```

(MPRIS is left out: it's for Linux desktops.)

### Ready-made builds
Every [release](https://github.com/iiKillish/tui-tunes/releases/latest) has builds for Linux
(x86_64, arm64), macOS (`macos-aarch64` for Apple Silicon, `macos-x86_64` for Intel) and Windows.
macOS may refuse to open a downloaded build; allow it with
`xattr -d com.apple.quarantine ./ttunes`.

Runtime dependencies:

- `dbus`, `libssl`
- `libpulse` (or `portaudio`, if built using the PortAudio backend)

## First run
ttunes needs a Spotify app of your own for Spotify's Web API. The first time it starts, it asks
for the app's client ID:

1. Create an app at https://developer.spotify.com/dashboard.
2. Add the redirect URI `http://127.0.0.1:8989/login`.
3. Choose the Web API.
4. Paste the app's client ID into ttunes and press <kbd>Enter</kbd>.

The ID is saved in the `client_id` file of the configuration directory (`~/.config/ttunes` on
Linux), so you're only asked once. Then ttunes opens two Spotify logins in your browser: one for
playback and one for your app (see [Authentication](#authentication)). A Spotify Premium account
is required. To use a different app later, set `client_id` in the configuration or delete the
saved file.

## Key Bindings
The keybindings listed below are configured by default. Additionally, if you
built `ttunes` with MPRIS support, you may be able to use media keys to control
playback depending on your desktop environment settings. Have a look at the
[configuration section](#configuration) if you want to set custom bindings.

<kbd>?</kbd> lists every key, grouped (playback, getting around, queue and library, other) and
saying what each does. Type to find a key or what it does, and press <kbd>Return</kbd> to run the
selected one.

To learn the keys, hold <kbd>Ctrl</kbd>+<kbd>Shift</kbd>: everything on screen that has a key
gets a label with it, like Vimium's link hints (the tabs, the player's buttons, the selected row or
card, a page's tabs). Letting go hides them. Holding <kbd>Ctrl</kbd>+<kbd>Shift</kbd> needs a
terminal that reports it (Kitty, Ghostty, WezTerm, foot);
<kbd>;</kbd> shows the labels in any terminal, until the next key.

On a Mac, the <kbd>Option</kbd> key does what <kbd>Alt</kbd> does here, in the same place: the
keys are the same as on Linux. ttunes turns the characters Option types (`˙` for Option+H) back
into Alt and the key.

### Count Prefixes
Like Vim, many keys can be prefixed with a number to repeat them that many
times. For example, <kbd>1</kbd><kbd>0</kbd><kbd>j</kbd> moves the selection
down 10 rows and <kbd>3</kbd><kbd>e</kbd> queues the next 3 items. The
count being typed is shown in the status bar, and is cancelled by
<kbd>Escape</kbd> or by opening the search/command prompt.

Counts apply to the movement keys, <kbd>e</kbd> and <kbd>.</kbd>, seeking
(<kbd>F</kbd>/<kbd>B</kbd> and their <kbd>Shift</kbd> variants), volume
(<kbd>+</kbd>/<kbd>-</kbd>/<kbd>[</kbd>/<kbd>]</kbd>), track skipping
(<kbd><</kbd>/<kbd>></kbd>) and search occurrences (<kbd>n</kbd>/<kbd>N</kbd>).
One-shot, toggle and menu commands (e.g. <kbd>O</kbd>, <kbd>A</kbd>,
<kbd>Return</kbd>, <kbd>R</kbd>, <kbd>Z</kbd>) ignore the count.

### Navigation
The window has the player at the top (or on the left of wide windows), then a row of screens:
_Home_, _Search_, _Library_, _Up next_ and _Lyrics_. Pages opened from a screen (an album, a
playlist, an artist) stack on top of it; the `‹ name` link above them goes back.

| Key                                                         | Command                                                                       |
|-------------------------------------------------------------|-------------------------------------------------------------------------------|
| <kbd>?</kbd>                                                | Show the keys.                                                                |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd> (hold) / <kbd>;</kbd>      | Show the keys of what's on screen.                                            |
| <kbd>,</kbd>                                                | Settings.                                                                     |
| <kbd>F1</kbd>…<kbd>F5</kbd>                                 | Home, Search, Library, Up next, Lyrics.                                       |
| <kbd>Tab</kbd> / <kbd>Shift</kbd>+<kbd>Tab</kbd>            | Next / previous screen (`screen next`/`screen previous`).                     |
| <kbd>Alt</kbd>+<kbd>L</kbd> / <kbd>Alt</kbd>+<kbd>H</kbd>   | Next / previous tab inside a page, e.g. Playlists → Albums (`tab next`/`tab previous`; also <kbd>Shift</kbd>+<kbd>→</kbd>/<kbd>←</kbd>). |
| <kbd>Shift</kbd>+<kbd>L</kbd>                               | Lyrics, or back to where you were.                                            |
| <kbd>Escape</kbd> / <kbd>Backspace</kbd>                    | Back to the previous page.                                                    |
| <kbd>/</kbd>                                                | Find in the current page (See [specific commands](#vim-like-search-bar)).     |
| <kbd>:</kbd>                                                | Open a Vim-like command prompt (See [specific commands](#vim-like-commands)). |
| <kbd>Q</kbd>                                                | Quit `ttunes`.                                                                |
| <kbd>g</kbd> / <kbd>G</kbd>                                 | Go to the top / bottom of the current page.                                   |

Everything can be clicked: tabs, cards, rows, the player's buttons, progress bar and volume (drag
its ● knob), and the Play / Shuffle / Like buttons on album, playlist and artist pages. In
terminals that support it (Kitty, Ghostty and foot), the mouse pointer turns into a hand over
things you can click, and a text cursor over text fields.

### Playback
| Key                           | Command                                                        |
|-------------------------------|----------------------------------------------------------------|
| <kbd>Return</kbd>             | Play track or playlist.                                        |
| <kbd>Space</kbd>              | Play / pause.                                                  |
| <kbd>E</kbd>                  | Queue track or playlist, then select the next one.             |
| <kbd>.</kbd>                  | Play the selected item after the currently playing track.      |
| <kbd>P</kbd>                  | Move to the currently playing track in the list or queue.      |
| <kbd>S</kbd>                  | Save the currently playing item to your library.               |
| <kbd>D</kbd>                  | Remove the currently playing item from your library.           |
| <kbd>Shift</kbd>+<kbd>P</kbd> | Play / pause, like <kbd>Space</kbd>.                           |
| <kbd>Shift</kbd>+<kbd>S</kbd> | Stop playback.                                                 |
| <kbd>Shift</kbd>+<kbd>U</kbd> | Update the library cache (tracks, artists, albums, playlists). |
| <kbd><</kbd>                  | Play the previous track.                                       |
| <kbd>></kbd>                  | Play the next track.                                           |
| <kbd>F</kbd>                  | Seek forward by 1 second.                                      |
| <kbd>Shift</kbd>+<kbd>F</kbd> | Seek forward by 10 seconds.                                    |
| <kbd>B</kbd>                  | Seek backward by 1 second.                                     |
| <kbd>Shift</kbd>+<kbd>B</kbd> | Seek backward by 10 seconds.                                   |
| <kbd>-</kbd>                  | Decrease volume by 1%.                                         |
| <kbd>+</kbd>                  | Increase volume by 1%.                                         |
| <kbd>[</kbd>                  | Decrease volume by 5%.                                         |
| <kbd>]</kbd>                  | Increase volume by 5%.                                         |
| <kbd>R</kbd>                  | Toggle _Repeat_ mode.                                          |
| <kbd>Z</kbd>                  | Toggle _Shuffle_ state.                                        |
| <kbd>Shift</kbd>+<kbd>H</kbd> | Like the currently playing track, or unlike it (the heart).    |
| <kbd>Shift</kbd>+<kbd>T</kbd> | Pause when the current track ends, or turn that off.           |

### Screens
- **Home**: a "Pick up where you left off" card, then what you've played lately, your top songs
  and artists. <kbd>H</kbd>/<kbd>L</kbd> switch between All, Albums, Playlists, Artists and Songs.
- **Search**: type and press <kbd>Return</kbd>; results open with an _All_ tab (top result, then
  rows of songs, artists, albums and playlists) and a tab per kind. Spotify links work too.
- **Library**: playlists (with _Liked Songs_ first), albums and artists as grids of covers, then
  saved tracks, podcasts and Spotify's categories.
- **Up next**: the queue, grouped by where songs were queued from. <kbd>Return</kbd> plays,
  <kbd>D</kbd> removes, <kbd>Shift</kbd>+<kbd>↑</kbd>/<kbd>↓</kbd> moves songs.
- **Lyrics**: time-synced lyrics from [LRCLIB](https://lrclib.net), following the song; click a line
  to jump to it.

### Settings
<kbd>,</kbd> (or `:settings`) opens the settings over the page, like the key list: which tabs the Library shows (turn off
Podcasts or Browse, say), the "Pick up where you left off" card on Home, autoplay, and the keys.
<kbd>Return</kbd> changes the selected setting; on a key, <kbd>Return</kbd> then the new key
rebinds it (its old keys stop doing it), and <kbd>D</kbd> gives it its default keys back. Changes
are saved to `config.toml`, keeping what's in it, and work at once.

### Context Menus
| Key                           | Command                                                                                                   |
|-------------------------------|-----------------------------------------------------------------------------------------------------------|
| <kbd>O</kbd>                  | Open a detail view or context for the **selected item**.                                                  |
| <kbd>Shift</kbd>+<kbd>O</kbd> | Open a context menu for the **currently playing track**.                                                  |
| <kbd>A</kbd>                  | Open the **album view** for the selected item.                                                            |
| <kbd>Shift</kbd>+<kbd>A</kbd> | Open the **artist view** for the selected item.                                                           |
| <kbd>Ctrl</kbd>+<kbd>V</kbd>  | Open the context menu for a Spotify link in your clipboard (if built with the `share_clipboard` feature). |
| <kbd>Backspace</kbd>          | Close the current view.                                                                                   |

When pressing <kbd>O</kbd>:

- If the _selected item_ is **not** a track, it opens a detail view.
- If the _selected item_ **is** a track, it opens a context menu with:
  - "Artist(s)" (let's you show or (un)follow a track's artist(s))
  - "Show Album"
  - "Share" (if built with the `share_clipboard` feature)
  - "Add to playlist" (type to filter your playlists)

### Sharing
(if built with the `share_clipboard` feature)

| Key                           | Command                                                                  |
|-------------------------------|--------------------------------------------------------------------------|
| <kbd>X</kbd>                  | Copy the URL to the **currently selected item** to the system clipboard. |
| <kbd>Shift</kbd>+<kbd>X</kbd> | Copy the URL to the **currently playing track** to the system clipboard. |

### Queue
| Key                          | Command                              |
|------------------------------|--------------------------------------|
| <kbd>C</kbd>                 | Clear the entire queue.              |
| <kbd>D</kbd>                 | Delete the currently selected track. |
| <kbd>Shift</kbd>+<kbd>↑</kbd>/<kbd>↓</kbd> | Move the selected track up or down. |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | Save the current queue.              |

### Library
Playlists (with a _Liked Songs_ card first), albums and artists are shown as grids of covers:
<kbd>←</kbd><kbd>↑</kbd><kbd>↓</kbd><kbd>→</kbd> (<kbd>H</kbd><kbd>J</kbd><kbd>K</kbd><kbd>L</kbd>)
move between cards, <kbd>Return</kbd> opens one. <kbd>Alt</kbd>+<kbd>H</kbd>/<kbd>L</kbd> or
<kbd>Shift</kbd>+<kbd>←</kbd>/<kbd>→</kbd> (`tab previous`/`tab next`) switch tabs anywhere. Search results
start with an _All_ tab: the top result, then rows of songs, artists, albums and playlists.

| Key          | Command                                 |
|--------------|-----------------------------------------|
| <kbd>D</kbd> | Delete the currently selected playlist. |

On one of your playlists' pages, <kbd>D</kbd> removes the selected track and
<kbd>Shift</kbd>+<kbd>↑</kbd>/<kbd>↓</kbd> moves it; `:rename` renames the playlist.

### Vim-Like Search Bar
| Key          | Command                     |
|--------------|-----------------------------|
| <kbd>n</kbd> | Previous search occurrence. |
| <kbd>N</kbd> | Next search occurrence.     |

### Vim-Like Commands
You can open a Vim-style command prompt using <kbd>:</kbd>,
trigger command completion by pressing <kbd>Tab</kbd>
and close it at any time with <kbd>Escape</kbd>.

Every command is listed below, grouped by what it does.

Note: \<FOO\> - mandatory arg; [BAR] - optional arg

#### Playback

| Command | Action |
|---------|--------|
| `playpause`<br/>Aliases: `pause`, `toggleplay`, `toggleplayback` | Toggle playback. |
| `stop` | Stop playback. |
| `previous` | Play the previous track. |
| `next` | Play the next track. |
| `play` | Play the selected item. |
| `queue` | Add the selected item to the end of the queue. |
| `playnext` | Play the selected item after the current track. |
| `seek` [`+`\|`-`]\<TIME\> | Seek to the specified position, or seek relative to current position by prepending `+`/`-`.<br/>\* TIME is a number of milliseconds, or a duration such as `1m 30s`, `90s` or `1.5 min` (units: `ms`, `s`, `m`, `h`). |
| `volup` [AMOUNT] | Raise the volume by AMOUNT percent (default 1). |
| `voldown` [AMOUNT] | Lower the volume by AMOUNT percent (default 1). |
| `repeat` [REPEAT_MODE]<br/>Alias: `loop` | Set repeat mode. Omit argument to step through the available modes.<br/>\* Valid values for REPEAT_MODE: `list` (aliases: `playlist`, `queue`), `track` (aliases: `once`, `single`), `none` (alias: `off`) |
| `shuffle` [`on`\|`off`] | Enable or disable shuffle. Omit argument to toggle. |
| `sleep` [\<TIME\>\|`track`\|`off`] | Set the sleep timer, which pauses playback. `track` pauses when the current track ends, and the next one waits paused; `off` turns the timer off. Omit the argument to toggle `track`.<br/>\* TIME is a number of minutes, or a duration such as `1h 30m`, `45m` or `90s` (units: `ms`, `s`, `m`, `h`). |
| `clear` | Clear the queue. |

#### Library and playlists

| Command | Action |
|---------|--------|
| `like` | Save the currently playing track to your library, or remove it if it's saved. |
| `save [current]` | Save selected item, if `current` is passed the currently playing item will be saved |
| `add [current]` | Add selected track to playlist, if `current` is passed the currently playing track will be added |
| `delete` | Remove the selected track from the playlist or the queue, or delete the selected playlist. |
| `newplaylist` \<NAME\> | Create a new playlist. |
| `rename` [NAME] | Rename the open playlist, or the one selected in the library. Omit NAME to type it in a dialog. Only your own and collaborative playlists can be renamed. |
| `sort` \<SORT_KEY\> [SORT_DIRECTION] | Sort a playlist. `original` goes back to the playlist's own order.<br/>\* Valid values for SORT_KEY: `title`, `album`, `artist`, `duration`, `added`, `original`<br/>\* Valid values for SORT_DIRECTION: `ascending` (default; aliases: `a`, `asc`), `descending` (aliases: `d`, `desc`) |
| `shift` `up`\|`down` [AMOUNT] | Move the selected track up or down by AMOUNT places (default 1), in the queue or in one of your playlists (on Spotify too). A playlist must be in its own order (`sort original`). |
| `update` | Refetch the library from Spotify: liked songs, albums, artists and playlists. |

#### Navigation

| Command | Action |
|---------|--------|
| `focus` \<SCREEN\> | Switch to a different view.<br/>\* Valid values for SCREEN: `home`, `search`, `library`, `queue`, `lyrics` |
| `screen` [`next`\|`previous`] | Switch to the next (default) or previous main screen. |
| `tab` [`next`\|`previous`] | Switch to the next (default) or previous tab, e.g. from Playlists to Albums in the library. |
| `panel` `queue`\|`lyrics` | `queue` shows the _Up next_ screen, `lyrics` shows the _Lyrics_ screen (or goes back to the previous screen if it's showing). |
| `move` \<DIRECTION\> \<STEP_SIZE\> | Scroll the current view `up`/`down`/`left`/`right` with integer step sizes, or `pageup`/`pagedown`/`pageleft`/`pageright` with float step sizes. |
| `open` `selected`\|`current` | Open the selected item, or the context menu of the currently playing track. |
| `goto` `album`\|`artist` | Open the album or artist of the selected item. |
| `back` | Close the open page and go back to the one under it. |
| `search` \<SEARCH\> | Search for a song/artist/album/etc. |
| `jump` \<TEXT\> | Select the next item in the list that matches TEXT, like <kbd>/</kbd>. |
| `jumpnext` | Select the next match of the last `jump`. |
| `jumpprevious` | Select the previous match of the last `jump`. |
| `insert` [URL] | Open a Spotify link: URL, or the link on the clipboard if built with the `share_clipboard` feature. |
| `share` \<ITEM\> | Copy a shareable URL of the item to the system clipboard. Requires the `share_clipboard` feature.<br/>\* Valid values for ITEM: `selected`, `current` |

#### Other

| Command | Action |
|---------|--------|
| `help` | Show the keys, grouped and in words; type to find one, and press <kbd>Return</kbd> to run it. |
| `hints` | Show the keys of what's on screen next to it, until the next key (like holding <kbd>Ctrl</kbd>+<kbd>Shift</kbd>). |
| `settings` | Open the settings: library tabs, the Home card, autoplay and keys. |
| `quit`<br/>Aliases: `q`, `x` | Quit `ttunes`. |
| `logout` | Remove any cached credentials from disk and quit `ttunes`. |
| `reload` | Reload the configuration from disk. See [Configuration](#configuration). |
| `reconnect` | Reconnect to Spotify (useful when session has expired or connection was lost) |
| `redraw` | Redraw the whole screen. |
| `exec` \<CMD\> | Execute a command in the system shell.<br/>\* Command output is printed to the terminal, so redirection (`2> /dev/null`) may be necessary. |
| `noop` | Do nothing. Useful for disabling default keybindings. See [custom keybindings](#custom-keybindings). |

## Remote control (IPC)
Apart from MPRIS, ttunes will also create a domain socket on UNIX platforms (Linux, macOS, *BSD).
The socket will be created in the platform's runtime directory. Run `ttunes info` to show the
location of this directory on your platform. Applications or scripts can connect to this socket to
send commands or be notified of the currently playing track, i.e. with `netcat`:

```
% nc -U $TTUNES_CACHE_DIRECTORY/ttunes.sock
play
{"mode":{"Playing":{"secs_since_epoch":1672249086,"nanos_since_epoch":547517730}},"playable":{"type":"Track","id":"2wcrQZ7ZJolYEfIaPP9yL4","uri":"spotify:track:2wcrQZ7ZJolYEfIaPP9yL4","title":"Hit Me Where It Hurts","track_number":4,"disc_number":1,"duration":184132,"artists":["Caroline Polachek"],"artist_ids":["4Ge8xMJNwt6EEXOzVXju9a"],"album":"Pang","album_id":"4ClyeVlAKJJViIyfVW0yQD","album_artists":["Caroline Polachek"],"cover_url":"https://i.scdn.co/image/ab67616d0000b2737d983e7bf67c2806218c2759","url":"https://open.spotify.com/track/2wcrQZ7ZJolYEfIaPP9yL4","added_at":"2022-12-19T22:41:05Z","list_index":0}}
playpause
{"mode":{"Paused":{"secs":25,"nanos":575000000}},"playable":{"type":"Track","id":"2wcrQZ7ZJolYEfIaPP9yL4","uri":"spotify:track:2wcrQZ7ZJolYEfIaPP9yL4","title":"Hit Me Where It Hurts","track_number":4,"disc_number":1,"duration":184132,"artists":["Caroline Polachek"],"artist_ids":["4Ge8xMJNwt6EEXOzVXju9a"],"album":"Pang","album_id":"4ClyeVlAKJJViIyfVW0yQD","album_artists":["Caroline Polachek"],"cover_url":"https://i.scdn.co/image/ab67616d0000b2737d983e7bf67c2806218c2759","url":"https://open.spotify.com/track/2wcrQZ7ZJolYEfIaPP9yL4","added_at":"2022-12-19T22:41:05Z","list_index":0}}
```

Each time the playback status changes (i.e. after sending the `play`/`playpause`
command or simply by playing the queue), the current status will be published as
a JSON structure.

Possible use cases for this could be:
- Controlling a detached ttunes session (in `tmux` for example)
- Displaying the currently playing track in your favorite application/status bar (see below)
- Setting up routines, i.e. to play specific songs/playlists when ttunes starts

### Extracting info on currently playing song
Using `netcat` and the domain socket, you can query the currently playing track
and other relevant information. Note that not all `netcat` versions are suitable,
as they typically tend to keep the connection to the socket open. OpenBSD's
`netcat` offers a work-around: by using the `-W` flag, it will close after a
specific number of packets have been received.

```
% nc -W 1 -U $TTUNES_CACHE_DIRECTORY/ttunes.sock
{"mode":{"Playing":{"secs_since_epoch":1675188934,"nanos_since_epoch":50913345}},"playable":{"type":"Track","id":"5Cp6a1h2VnuOtsh1Nqxfv6","uri":"spotify:track:5Cp6a1h2VnuOtsh1Nqxfv6","title":"New Track","track_number":1,"disc_number":1,"duration":498358,"artists":["Francis Bebey"],"artist_ids":["0mdmrbu5UZ32uRcRp2z6mr"],"album":"African Electronic Music (1975-1982)","album_id":"7w99Aae1tYSTSb1OiDnxYY","album_artists":["Francis Bebey"],"cover_url":"https://i.scdn.co/image/ab67616d0000b2736ab57cedf27177fae1eaed87","url":"https://open.spotify.com/track/5Cp6a1h2VnuOtsh1Nqxfv6","added_at":"2020-12-22T09:57:17Z","list_index":0}}
```

This results in a single output in `JSON` format, which can e.g. be parsed using [jq](https://stedolan.github.io/jq/).
For example, you can get the currently playing artist and title in your
terminal as follows:

```
% nc -W 1 -U $TTUNES_CACHE_DIRECTORY/ttunes.sock | jq '.playable.title'
"PUMPIN' JUMPIN'"

% nc -W 1 -U $TTUNES_CACHE_DIRECTORY/ttunes.sock | jq '.playable.artists[0]'
"Hideki Naganuma"
```

## Configuration
Configuration is saved to the `config.toml` file in the platform's standard configuration directory.
Run `ttunes info` to show the location of this directory on your platform. To reload the
configuration during runtime use the `reload` command.

Possible configuration values are:

| Name                            | Description                                                    | Possible values                                                                       | Default             |
|---------------------------------|----------------------------------------------------------------|---------------------------------------------------------------------------------------|---------------------|
| `command_key`                   | Key to open command line                                       | Single character                                                                      | `:`                 |
| `initial_screen`                | Screen to show after startup                                   | `"home"`, `"search"`, `"library"`, `"queue"`, `"lyrics"`                            | `"home"`            |
| `use_nerdfont`                  | Turn nerdfont glyphs on/off                                    | `true`, `false`                                                                       | `false`             |
| `backend`                       | Audio backend to use                                           | String<sup>[1]</sup>                                                                  |                     |
| `backend_device`                | Audio device to configure the backend                          | String                                                                                |                     |
| `audio_cache`                   | Enable caching of audio files                                  | `true`, `false`                                                                       | `true`              |
| `audio_cache_size`              | Maximum size of audio cache in MiB                             | Number                                                                                |                     |
| `volnorm`                       | Enable volume normalization                                    | `true`, `false`                                                                       | `false`             |
| `volnorm_pregain`               | Normalization pregain to apply in dB (if enabled)              | Number                                                                                | `0.0`               |
| `default_keybindings`           | Enable default keybindings                                     | `true`, `false`                                                                       | `false`             |
| `notify`<sup>[2]</sup>          | Enable desktop notifications                                   | `true`, `false`                                                                       | `false`             |
| `bitrate`                       | Audio bitrate to use for streaming                             | `96`, `160`, `320`                                                                    | `320`               |
| `gapless`                       | Enable gapless playback                                        | `true`, `false`                                                                       | `true`              |
| `shuffle`                       | Set default shuffle state                                      | `true`, `false`                                                                       | `false`             |
| `repeat`                        | Set default repeat mode                                        | `"off"`, `"track"`, `"playlist"`                                                      | `"off"`             |
| `playback_state`                | Set default playback state                                     | `"Stopped"`, `"Paused"`, `"Playing"`, `"Default"`                                     | `"Paused"`          |
| `library_tabs`                  | Tabs to show in library screen                                 | Array of `"tracks"`, `"albums"`, `"artists"`, `"playlists"`, `"podcasts"`, `"browse"` | All tabs            |
| `hide_display_names`            | Hides spotify usernames in the library header and on playlists | `true`, `false`                                                                       | `false`             |
| `[track_format]`                | Set active fields shown in Library/Queue views                 | See [track formatting](#track-formatting)                                             |                     |
| `[notification_format]`         | Set the text displayed in notifications<sup>[2]</sup>          | See [notification formatting](#notification-formatting)                               |                     |
| `[theme]`                       | Custom theme                                                   | See [custom theme](#theming)                                                          |                     |
| `[keybindings]`                 | Custom keybindings                                             | See [custom keybindings](#custom-keybindings)                                         |                     |
| `ap_port`                       | Set ap-port for librespot (for restrictive firewalls)          | `80`, `443`, `4070`                                                                   |                     |
| `client_id`                     | Client ID of your own Spotify app for Web API requests<sup>[3]</sup> | String                                                                          | The ID saved at first run |
| `autoplay`                      | Keep playing similar songs when the queue runs out<sup>[4]</sup> | `true`, `false`                                                                     | `true`              |
| `pick_up_where_you_left_off`    | Show the "Pick up where you left off" card on Home             | `true`, `false`                                                                       | `true`              |
| `image_protocol`                | How to draw covers; see [cover drawing](#cover-drawing)        | `"auto"`, `"kitty"`, `"sixel"`, `"iterm2"`, `"none"`                                  | `"auto"`            |

1. Run `ttunes -h` for a list of devices.
2. If built with the `notify` feature.
3. Optional: overrides the ID saved by the [first run](#first-run) setup in the `client_id` file
   of the configuration directory. The app needs the redirect URI `http://127.0.0.1:8989/login`.
4. When the last song in the queue starts, ttunes asks Spotify's autoplay for songs like it (based
   on its album and the songs played before it) and adds them to the end of the queue, shown as
   "Next from: Autoplay". Songs you queue go ahead of them. Not with repeat on, and not for
   podcasts.

### Custom Keybindings
Keybindings can be configured in `[keybindings]` section in `config.toml`.

Each key-value pair specifies one keybinding, where the key is a string in the
format of:

```
[MODIFIER+]<CHAR|NAMED_KEY>
where:
  MODIFIER: Shift|Alt|Ctrl
  CHAR: Any printable character
  NAMED_KEY: Enter|Space|Tab|Backspace|Esc|Left|Right|Up|Down
    |Ins|Del|Home|End|PageUp|PageDown|PauseBreak|NumpadCenter
    |F0|F1|F2|F3|F4|F5|F6|F7|F8|F9|F10|F11|F12
```

For implementation see [commands::CommandManager::parse_key](/src/commands.rs).

Its value is a string that can be parsed as a command. See
[Vim-Like Commands](#vim-like-commands).

<details>
  <summary>Examples: (Click to show/hide)</summary>

```toml
[keybindings]
# Bind "Shift+i" to "Seek forward 10 seconds"
"Shift+i" = "seek +10s"
```

To disable a default keybinding, set its command to `noop`:

```toml
# Use "Shift+q" to quit instead of the default "q"
[keybindings]
"Shift+q" = "quit"
"q" = "noop"
```

</details>

### Proxy

All of ttunes' network traffic (the Spotify access point, the Spotify APIs,
the OAuth login flow and cover art downloads) can be routed through a proxy
configured in the configuration file:

```toml
# HTTP CONNECT proxy (optionally with credentials)
proxy = "http://user:password@foo.bar:4444"

# SOCKS5 proxy with remote DNS resolution (the proxy resolves hostnames)
proxy = "socks5h://foo.bar:1080"
proxy = "socks5h://user:password@foo.bar:1080"
```

`socks5://` is accepted as an alias for `socks5h://`; in both cases the
hostname is resolved by the proxy, which is usually what you want when local
DNS is unusable.

Alternatively, the legacy behavior is still supported: without a `proxy`
setting, ttunes honours the `http_proxy` environment variable (HTTP CONNECT
proxies only).

```sh
# In sh-like shells
http_proxy="http://foo.bar:4444" ttunes
```

### Theming
ttunes has no colors of its own: it uses your terminal's colors, so it matches whatever theme
you already use. Text is in the terminal's foreground on its background, the accent is ANSI blue
and dimmed text is ANSI bright black. At startup ttunes asks the terminal for its real
background and foreground, and derives the player's and the selected row's background from them:
the background moved about 7% towards the foreground, so lighter on dark themes and darker on
light ones. Terminals that don't answer (older ones, tmux without passthrough) get the terminal's
background there, and the selected row is still marked with a `▌` in the accent.

Any color can be set in the optional `[theme]` section. Colors are a name (`black`, `red`,
`green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, the same with `light ` in front, e.g.
`light blue`), `#rrggbb`, `#rgb`, or `default` for the terminal's own color.

| Key                     | Used for                                                        | Default                    |
|-------------------------|-----------------------------------------------------------------|----------------------------|
| `background`            | The main background                                             | `default`                  |
| `primary`               | Normal text                                                     | `default`                  |
| `secondary`             | Dimmed text: artists, lengths, hints                            | `light black`              |
| `title`                 | The accent: titles, the current tab, active toggles             | `blue`                     |
| `playing`               | The track that's playing                                        | `blue`                     |
| `playing_bg`            | Background of the track that's playing                          | `background`               |
| `highlight`             | Text on filled buttons                                          | The terminal's background  |
| `highlight_bg`          | Filled buttons (Play)                                           | `blue`                     |
| `highlight_inactive_bg` | Selection in menus without focus, empty cover squares           | Derived, or `light black`  |
| `statusbar_bg`          | The player, menus and the selected row                          | Derived, or `default`      |
| `statusbar_progress`    | The filled part of the progress and volume bars                 | `blue`                     |
| `statusbar_progress_bg` | The empty part of the progress and volume bars                  | `light black`              |
| `error`                 | Error messages                                                  | `red`                      |
| `error_bg`              | Background of error messages                                    | `background`               |
| `cmdline`               | Text on the command line                                        | `default`                  |
| `cmdline_bg`            | Background of the command line                                  | `background`               |

Setting `background` to a `#rrggbb` color derives the surfaces from it instead of the terminal's
background. For example, to look like Spotify's official client:

```toml
[theme]
background = "#121212"
primary = "#ffffff"
secondary = "#b3b3b3"
title = "#1db954"
playing = "#1db954"
highlight_bg = "#1db954"
statusbar_progress = "#1db954"
```

### Track Formatting
It's possible to customize how tracks are shown in list views with `[track_format]`.
If you don't define `center` for example, the default value will be used.
Available options for tracks: `%artists`, `%artist`, `%title`, `%album`, `%saved`,
`%duration`.
`%artists` will show all contributing artists, while `%artist` only shows the first listed artist.

Default configuration:

```toml
[track_format]
left = "%artists - %title"
center = "%album"
right = "%saved %duration"
```

<details>
  <summary>Examples: (Click to show/hide)</summary>

Example 1 - Show only album name and track name after it:

```toml
[track_format]
left = "%album"
center = "%title"
right = ""
```

Example 2 - Show track title before artists, and don't show album at all:

```toml
[track_format]
left = "%title - %artists"
center = ""
```

Example 3 - Show everything as default, but hide saved status and track length:

```toml
[track_format]
right = ""
```

Example 4 - Show everything as default, except show title before artists:

```toml
[track_format]
left = "%title - %artists"
```

Example 5 - Show saved status and duration first, followed by track title and artists, with the album last:

```toml
[track_format]
left = "|%saved| %duration | %title - %artists"
center = ""
right = "%album"
```

</details>

### Notification Formatting
`ttunes` also supports customizing the way notifications are displayed
(which appear when compiled with the `notify` feature and `notify = true`).
The title and body of the notification can be set, with `title` and `body`, or the default will be used.
The formatting options are the same as those for [track formatting](#track-formatting) (`%artists`, `%title`, etc)

Default configuration:

```toml
[notification_format]
title = "%title"
body = "%artists"
```

### Cover Drawing
With the `cover` feature (on by default), `ttunes` draws album art (in the player, on page
headers and next to list items) and the player's control icons with whichever image protocol the
terminal speaks:

- the Kitty graphics protocol: Kitty, Ghostty, WezTerm, Konsole
- sixel: foot, Windows Terminal (running ttunes in WSL), mlterm, Contour, mintty
- iTerm2's inline images: iTerm2

The terminal is recognized from environment variables such as `TERM` and `TERM_PROGRAM`. Other
terminals with sixel support, such as xterm (started with `-ti vt340`) or VS Code (with
`terminal.integrated.enableImages`), aren't recognized this way: set `image_protocol` in the
configuration to use them, or to `"none"` to turn images off. In terminals without images, no
covers are shown.

Sixel and iTerm2 images are drawn at the terminal's cell size in pixels, which ttunes asks the
terminal for; terminals that don't report it get images of the wrong size.

## Authentication
`ttunes` uses OAuth2 for authentication. When launched for the first time, a link will be generated
that can be opened in any browser. After logging in on the displayed page, you can start to use
`ttunes`. The OAuth2 flow is the only supported one, as username/password authentication has been
removed by Spotify.

A second login authorizes your own Spotify app (see [First run](#first-run)) for the Web API, which
ttunes uses for the library, search and playlists. Its token is stored in
`rspotify_token_<client ID>.json` in the cache directory.

The credentials are stored in `librespot/credentials.json` in the user's cache directory. Run
`ttunes info` to show the location of this directory.

The `logout` command can be used to remove cached credentials. See
[Vim-Like Commands](#vim-like-commands).
