# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A _Local_ tab in the Library plays the MP3 and FLAC files in your music
  folder (`~/Music`, or the folders in the new `music_dirs` setting), with the
  queue, seeking, lyrics and MPRIS like Spotify songs. It shows the folders as
  they are on disk: Enter opens one as a page, with Play and Shuffle for all
  of it. Covers come from the files, or a `cover.jpg` (and the like) in the
  folder

- A setting (`queue_click`, also in the settings under Playback) to have
  clicking a song in _Up next_ open its menu instead of playing it

### Fixed

- Local files in playlists no longer all show as playing when one of them is,
  and show their artists in the player and notifications

## [0.6.0]

### Changed

- The command line lists the commands that fit what's typed, with their
  arguments and what they do, then the values a command takes (with their
  keys); Tab completes and goes through them, as do the arrows and clicks
- The key list (`?`) shows one section at a time (←/→ or Tab for the next),
  lists the most used keys first, and has a Commands section with every command;
  typing still searches all of them, with how many match in each section
- Ctrl+C quits, like `q`, even while typing in the command line or a search
  field (rebindable as `Ctrl+c`)
- Keys are changed in the key list (Ctrl+E on one, Ctrl+R for its default
  keys) instead of the settings
- The settings are in sections (General, Playback, Appearance) and have more in
  them: the screen to start on, notifications, sound quality, gapless playback,
  loudness, volume steps, the audio cache, Nerd Font icons, how covers are
  drawn, a theme to pick (Catppuccin, Gruvbox, Nord, Dracula, Tokyo Night, Rosé
  Pine) and every color

### Fixed

- Covers no longer flicker when a menu opens or closes over them: with Kitty
  graphics they're sent once and moved or cut in place, and with sixel and
  iTerm2 they're no longer blanked before being drawn again

## [0.5.1]

### Fixed

- Skipping to another song no longer plays a moment of the old one first,
  pausing on Linux stops at once, and resuming picks up exactly where it
  paused instead of skipping ahead a little

### Changed

- The logins, the listening history behind Stats, the home screen's history and
  the volume are kept in the data directory (`~/.local/share/ttunes` on Linux)
  instead of the cache, so clearing the cache no longer logs out or loses them;
  they're moved there on start
- Songs played again count as recently used in the audio cache across
  restarts, so they aren't deleted first when it's full (filesystems mounted
  `noatime` didn't record it)
- The audio cache is limited to 5 GiB by default (`audio_cache_size`); it used
  to grow without limit
- Windows builds with a plain `cargo build` / `cargo install`, covers included,
  like Linux and macOS: no features to pick per system

## [0.5.0]

### Security

- The debug log (`-d`) no longer contains the Spotify login token or proxy
  passwords, and only you can read it
- The saved Spotify login, the cache and the configuration folders can only
  be read by you; ones made by older versions are tightened on start
- Logging out also forgets the Web API login
- Commands sent to the IPC socket can't run shell commands (`exec`)
- Releases are built without access to the repository, and the actions CI
  uses are pinned

### Changed

- Opening a song's menu, an album, a podcast, a category or a link, and
  playing an album, playlist or artist, no longer freeze the interface while
  Spotify answers
- Covers are decoded in the background, thumbnails download a small size, at
  most four download at once, and failed downloads wait a minute before
  trying again instead of retrying many times a second
- ttunes redraws only as often as something changes, instead of four times a
  second when idle
- Fewer requests to Spotify, so it rate limits less: one per search instead of
  six, none for autoplay's songs, artist pictures remembered between runs,
  Home reusing what it saved, and followed artists loaded in full only when
  they changed
- The login token is renewed before it expires, once, and requests wait for it
- Library caches are written whole and only when they change
- Smaller release binaries, and a faster build without a second TLS library

### Fixed

- A rare freeze when liking a song while a list was being drawn
- A song's menu (<kbd>o</kbd>) covered the player's buttons; menus and prompts
  open over the page when there's room
- Pressing <kbd>o</kbd> again in a song's menu played the song instead of
  closing the menu

## [0.4.4]

### Changed

- Home remembers the albums, playlists and artists you played from, up to 20.
  Spotify only lists the last 50 songs played, which can all be from one or two
  albums, so Home used to forget everything older

### Fixed

- When Spotify wouldn't give Home its recently played songs or top songs (it
  limits busy starts), Albums, Playlists and Songs said there was nothing there
  for ten minutes. They say Spotify couldn't be reached, and Home tries again
  after 30 seconds

## [0.4.3]

### Changed

- Searching ignores accents: "beyonce" finds "Beyoncé", in lists and on cards
  (<kbd>/</kbd>) and when picking a playlist to add to

## [0.4.2]

### Changed

- The loading screen stays up for two seconds, so it doesn't flash by; any key
  goes on to the app as soon as it's ready

## [0.4.1]

### Changed

- `cargo install --locked ttunes` works on macOS, playing through CoreAudio:
  each system gets its own sound backend (PulseAudio on Linux and the BSDs,
  Rodio on macOS) without features. The `pulseaudio_backend` feature does
  nothing now; Linux builds always have PulseAudio, which PipeWire provides

### Fixed

- Installing 0.4.0 failed with Rust 1.93. ttunes says which Rust it needs
  (`rust-version`), and CI builds with it
- Pressing <kbd>Space</kbd> while a song was loading skipped to the next song.
  The player says "loading…", and <kbd>Space</kbd> decides whether the song
  starts once it's loaded
- When Spotify refused to play songs (it does when asked for too many at once),
  ttunes skipped through the whole queue. It stops after three in a row and
  says so

## [0.4.0]

### Added

- Install instructions for macOS, which needs the Rodio backend, and for the
  ready-made release builds
- A loading screen while ttunes connects to Spotify: "ttunes" typed out in big
  letters, with what it's doing
- Hold <kbd>Ctrl</kbd>+<kbd>Shift</kbd> (in Kitty, Ghostty, WezTerm or foot) or
  press <kbd>;</kbd> to label what's on screen with its key, like Vimium
- Settings, in a popup like the key list (<kbd>,</kbd> or `settings`): library tabs, the "Pick up
  where you left off" card (also the `pick_up_where_you_left_off` option),
  autoplay, and rebinding keys, saved to config.toml
- A Stats screen (<kbd>F6</kbd>): time listened, streaks, when you listen and
  your top songs and artists as ttunes notes them, Spotify's top artists and
  songs over 4 weeks, 6 months and all time, and your library in numbers
- Covers and control icons in terminals with sixel (foot, Windows Terminal
  with WSL, mlterm, Contour, mintty) or iTerm2's inline images (iTerm2), besides the
  Kitty graphics protocol. The `image_protocol` option picks one by hand, or
  turns images off
- Move tracks in your own and collaborative playlists with `shift` or
  <kbd>Shift</kbd>+<kbd>↑</kbd>/<kbd>↓</kbd>, as in the queue
- `rename` renames the open playlist, or the one selected in the library
- `sort original` puts a sorted playlist back in its own order
- Playlists Spotify makes (Daily Mix, Discover Weekly, Release Radar, editorial
  playlists) open and play again, from Home and from links (`insert`): the Web
  API no longer gives them to apps, so they're loaded the way Spotify's own
  apps load them
- Autoplay: when the queue runs out, similar songs keep playing, as in
  Spotify's apps. They're added when the last song starts, shown as "Next
  from: Autoplay", and songs you queue go ahead of them. Turn it off with
  `autoplay = false`; it's off while repeat is on

### Changed

- <kbd>Space</kbd> plays and pauses, as in Spotify's apps; <kbd>E</kbd> queues
  the selected item (what <kbd>Space</kbd> did)
- Queueing an album or playlist no longer freezes the interface while its songs
  load
- The key list (<kbd>?</kbd>) is grouped and says what each key does in words,
  with keys doing the same on one row
- On a Mac, Option with a key works as Alt does on Linux, in the same place
- Home no longer greets you by name
- Playlist pages open at once and show "Loading…" until their songs are there,
  instead of freezing the interface
- Moving and removing songs in a playlist shows at once; Spotify is updated in
  the background, and a change it refuses is reported
- The songs of playlists Spotify makes are kept on disk and only loaded again
  once Spotify has changed the playlist

- The macOS release builds include album covers, clipboard sharing and
  notifications, like the Linux ones

### Fixed

- Pausing with the Rodio backend (macOS, Windows) lagged half a second or so:
  it waited for the buffered audio to play out
- Artist pages showing no songs, so Play played nothing: Spotify stopped giving
  apps artists' top tracks, which are now loaded the way its own apps load them

## [0.3.1]

### Fixed

- `cargo install ttunes` without `--locked` failing to build librespot-core:
  vergen is pinned to 9.0, as vergen 9.1 no longer works with the vergen-gitcl
  1.0 that librespot's build script uses

## [0.3.0]

### Added

- The mouse pointer turns into a hand over things that can be clicked, and a
  text cursor over text fields, in terminals that support it (Kitty, Ghostty,
  foot)

### Changed

- The selected tab is no longer underlined; it's still bold and in the accent
  color

### Fixed

- Clicking the space below the last item on the Home screen no longer opens the
  next item, which isn't shown
- Clicking just above a list, such as the blank row under a page's tabs, no
  longer selects or plays the list's top item
- Clicking an empty row in a filtered menu no longer loses the selection

## [0.2.0]

### Added

- Sleep timer: `sleep 30m` pauses playback after a while, and `sleep track`
  or <kbd>Shift</kbd>+<kbd>T</kbd> pauses when the current track ends, with
  the next one waiting paused. The player shows the time left
- Type to filter the playlists in "Add to playlist" and the key list
  (<kbd>?</kbd>)
- <kbd>P</kbd> says so when the playing track isn't in the list

### Changed

- `focus` with an unknown screen is an error when the command is read, so a
  typo in a key binding is reported like any other bad argument instead of
  only when the key is pressed
- The commands list in the user docs covers every command

### Fixed

- <kbd>Tab</kbd> on the Search screen switches to the next screen in one press,
  instead of first only leaving the search field

## [0.1.2]

### Changed

- Startup no longer downloads every liked song and saved album again when
  they haven't changed; `:update` still refetches everything

### Fixed

- Playback stopping when Spotify closes the connection: ttunes reconnects in
  the background and the current song keeps playing
- Playback dying on `:reconnect` an hour after logging in through the
  browser, as the login's access token had expired

### Security

- Replaced the unmaintained `parse_duration` crate, which a duration with a
  huge exponent could make use unbounded CPU and memory
  ([GHSA-qpgv-g792-wh6x](https://github.com/advisories/GHSA-qpgv-g792-wh6x)).
  `seek` durations now take numbers with the units `ms`, `s`, `m` and `h`
- Updated rustls to 0.23.46
  ([GHSA-2mjx-qc3c-rqvc](https://github.com/advisories/GHSA-2mjx-qc3c-rqvc))

## [0.1.1]

### Changed

- Album covers are on by default: `cargo install --locked ttunes` includes
  them, no `--features cover` needed
- The PulseAudio stream is described as "Spotify player for the terminal"
  instead of ncspot's "ncurses Spotify client"

## [0.1.0]

First release of ttunes, based on [ncspot](https://github.com/hrkfdn/ncspot)
1.5.0. The entries below list what changed since ncspot 1.5.0; the older
entries are ncspot's own history.

### Added

- A new interface written with [ratatui](https://ratatui.rs): a now-playing hero
  (big cover, track, progress, phone-style controls and a volume slider with a
  draggable knob) beside or above the _Home_, _Search_, _Library_, _Up next_
  and _Lyrics_ screens
- Home screen with a "Pick up where you left off" card, filter pills and a list
  of recent and top items
- Cover grids for the library and search results, with an _All_ tab in search
- Album, playlist and artist pages with a header tinted by the cover's color and
  Play / Shuffle / Like buttons
- Mouse support throughout the interface
- Covers and control icons drawn with the Kitty graphics protocol (Kitty,
  Ghostty, WezTerm) when built with the `cover` feature
- Time-synced lyrics from [LRCLIB](https://lrclib.net) on the _Lyrics_ screen
- Like the current song with the heart button, <kbd>Shift</kbd>+<kbd>H</kbd> or
  the `like` command
- `screen` command; <kbd>Tab</kbd>/<kbd>Shift</kbd>+<kbd>Tab</kbd> switch
  between the main screens and <kbd>F1</kbd>-<kbd>F5</kbd> open them in tab order
- `tab` command; <kbd>Alt</kbd>+<kbd>H</kbd>/<kbd>L</kbd> and
  <kbd>Shift</kbd>+arrows switch tabs inside a page
- First-run setup that asks for the client ID of your own Spotify app and saves
  it; the `client_id` configuration option overrides it
- `volume_curve` configuration option (`cubic`, `log` or `linear`)
- Show a countdown in the player when Spotify rate limits a request
- Remember where queued songs came from ("Next from") across restarts

### Changed

- Colors come from the terminal by default: its own foreground and background,
  an ANSI blue accent, and player and selection backgrounds derived from the
  background it reports. `[theme]` still overrides any color
- Renamed to ttunes: the binary, the config and cache directories
  (`~/.config/ttunes`, `~/.cache/ttunes`), the IPC socket and the MPRIS bus
  name `org.mpris.MediaPlayer2.ttunes`
- The ratatui interface replaces cursive; the `crossterm_backend`,
  `ncurses_backend`, `pancurses_backend` and `termion_backend` features are gone
- The volume slider uses a cubic curve by default, like system mixers
- Shuffle reorders the queue itself, so _Up next_ shows the real play order
- Search requests 10 results at a time, Spotify's new limit
- MPRIS announces capability changes, and `CanControl` is constant, so desktop
  media controls treat ttunes as controllable

### Removed

- ncspot's shared client ID: Web API requests always use your own Spotify app
- The `similar` command and its <kbd>M</kbd>/<kbd>Shift</kbd>+<kbd>M</kbd>
  keybindings, as Spotify no longer offers recommendations to third-party apps
- `panel focus`; `panel` now takes `queue` or `lyrics`
- Configuration options the new interface doesn't use: `flip_status_indicators`,
  `cover_max_scale`, `statusbar_format`, `now_playing_height`,
  `queue_panel_width` and the `playing_selected`, `statusbar` and
  `search_match` theme colors (old configs still load)

### Fixed

- Don't block the interface on long rate-limit waits
- Set the PulseAudio stream properties once at startup instead of on every
  (re)connect, which raced with other threads reading the environment

## ncspot history

## [1.5.0]

### Added

- Vim-like command completion by pressing <kbd>Tab</kbd>
- Support Vim-style numeric count prefixes for keybindings (e.g. `10j` moves
  down 10 rows); the pending count is shown in the status bar
- Add a `proxy` configuration option that routes all of ncspot's network
  traffic (access point, Spotify APIs, OAuth and cover art downloads) through
  a single proxy. Supports HTTP CONNECT proxies and SOCKS5 proxies with
  remote DNS resolution (`socks5h://`), both with optional credentials.
  Without this option, the `http_proxy` environment variable is still
  honoured as before.

### Changed

- Replace already existing queue notifications instead of stacking new ones.

## [1.4.0]

### Added

- Draw cover art directly in supported terminals using terminal image
  protocols, falling back to block rendering when unsupported

### Fixed

- Close the Search screen with the Escape key
- Stop discarding the Web API refresh token on every refresh, which caused
  a new OAuth2 login to be required daily

## [1.3.4]

### Changed

- Update `rspotify` to 0.16.1 and use Spotify's Library API for saving and following items
- Display podcast shows by name only, as Spotify removed publisher information from the Web API

### Fixed

- Fix crashing when attempting to add a song to a playlist
- Fix incorrect shuffle order after appending a track while shuffle is enabled

## [1.3.3]

### Fixed

- Fix token generation issues after Spotify API changes

### Added

- **Added new Vim motions** for moving to the top/bottom of a page (`g` and `G`)

## [1.3.2]

### Fixed

- Playlist retrieval crashing when list contains podcast episodes
- Crash when shifting a song by an amount greater than the queue's length
- Crash when displaying songs that do not have an (available) artist
- Playback broken due to Spotify API change

## [1.3.1]

### Fixed

- Bug preventing any type of playback due to spotify API changes.
- Bug preventing retrieval of new song metadata from spotify.

## [1.3.0]

### Added

- Automatically find free port for OAuth2 login flow

### Fixed

- Skip unplayable tracks
- Queue UI correctly plays a track when clicking on an already selected item
- Swap foreground and background colors for command line input
- Do not crash due to unavailable tracks in queue
- Do not fail when receiving playables of unknown types

## [1.2.2]

### Added

- `%artist` formatting option to only show single artist name
- Build release for aarch64 on Linux

### Removed

- Support for basic password managers as OAuth2 is now the only supported authentication flow

### Fixed

- Missing automatic man page generation for subcommands
- Bug causing forced password reset after using `ncpsot`

## [1.2.1] - 2024-10-31

### Fixed

- Fix error in fetching of window size leading to crashes with cover feature

## [1.2.0] - 2024-10-15

### Added

- Emit MPRIS `Seeked` signal

### Fixed

- Switch to OAuth2 login mechanism
- Uses librespot 0.5.0 which containts many bugfixes, including:
  - Files are downloaded via CDN, which should fix playback of some podcasts
  - Better access point connection handling, hopefully resulting in faster startups
  - Fix frequent disconnections for some users

## [1.1.2] - 2024-07-16

### Fixed

- Handling localized Spotify URLs
- Fetching playlists did not work for some users unless they set a playlist image

## [1.1.1] - 2024-05-10

### Added
- `ap_port` configuration variable to choose what Spotify access point port to use
- Instructions for installation with `snap`

### Fixed

- Not all requests respecting proxy settings as configured with environment variables
- MPRIS controls showing unknown artist/title on GNOME

## [1.1.0] - 2024-03-05

### Added

- Instructions for installation with `winget`
- Automatic macOS arm64 builds to releases

### Fixed

- Crash on Android (Termux) due to unknown user runtime directory
- Crash due to misconfigured or unavailable audio backend
- Volume changes from within `ncspot` not being reflected in the desktop notification
- Crash when `ncspot` tried to update a valid API token or the update failed
- Complete freeze when `ncspot` was running for a long time

## [1.0.0] - 2023-12-16

### Added

- Special color for unavailable items
- Changelog with all the relevant user-facing changes to the project
- `info` command line subcommand to show platform specific information
- Git commit hash to version string to simplify determining the exact version

### Changed

- Improve error messages generated by the command line
- Build with crossterm terminal backend by default
- Move UNIX IPC socket from the user's cache path to the user's runtime directory
- Improve messages relating to errors in the configuration file

### Fixed

- Crash when internal commands can't be handled
- Documentation for the behavior of the `Ctrl+S` keybinding
- Multiple instances interfering with each other's MPRIS implementation
- An unlikely crash when the UNIX IPC socket is removed before `ncspot` is closed
- Guaranteed crash while quiting `ncspot` when using MPRIS
- MPRIS volume not being updated when given numbers smaller than 0 or larger than 1
- Play previous song command not working when first item in the queue was playing

## [0.13.4] - 2023-07-24

### Added

- `save current` and `add current` commands for saving/adding the currently playing item
- Visual indicator for local tracks
- Error when deleting local tracks, as this is currently not possible


### Changed

- Improve release profile to generate higher quality executables
- Ignore a leading 'The' when sorting artists
- Improve context menu loading time

### Fixed

- Crash when deleting local tracks from a playlist

## [0.13.3] - 2023-06-11

### Added

- Instructions for installation with `cargo`

### Removed

- Instructions for installation with Snap as the package has been removed

### Fixed

- Disappearing selection in lists when removing the last element
- Playback notification not being shown on some systems
- IPC socket being overriden when opening multiple instances
- IPC socket persisting after closing `ncspot`
- Crash when submitting a command with a multibyte unicode character as the prefix
- Tabs switching when command line is active and `Left`/`Right` is held down

## [0.13.2] - 2023-05-04

### Added

- `ncurses_backend` feature flag to compile with `ncurses` support

### Fixed

- Crash when running with MPRIS support while D-Bus isn't available
- Nerdfont icons not rendering correctly

## [0.13.1] - 2023-04-05

### Added

- Vim-like page scrolling commands like `Ctrl+d`/`Ctrl+u` (not set to keybinding by default)
- Double click emulation to play items

### Changed

- Split up the documentation to make it easier to find relevant information

### Fixed

- Documentation about library tabs to include the new browse tab
- Incorrect D-Bus paths for the MPRIS implementation

## [0.13.0] - 2023-03-09

### Added

- Example for how to use the UNIX domain socket IPC interface
- `HighlightInactive` option to the theme config
- `reconnect` command to reconnect to Spotify if network changes broke the current session
- Support for basic password managers
- Man page support
- Shell completion support

### Changed

- Improve the way duration for items is displayed
- Remove `pulseaudio` dependency from the Debian package

### Fixed

- `Ctrl+z` shortcut not working to put `ncspot` in the background
- Cover not being sent to the notification if not compiled with the `cover` feature

## [0.12.0] - 2022-12-29

### Added

- Context menu action to save/remove the album of the selected track to the library
- Flatpak option to installation instructions
- IPC socket on UNIX platforms to control `ncspot` remotely

### Fixed

- Crash information being dumped in the terminal. It is now automatically written to a file instead.
- Bug causing command prompt to appear after 'Connecting to Spotify..' startup message after quiting
  `ncspot`

## [0.11.2] - 2022-10-15

### Added

- More context menu options to items to more closely match what's already offered for tracks
- POSIX signal handling to quit gracefully when not closed by the user (eg. system shutdown)

### Changed

- Move playback controls out of their own submenu in the context menu

### Fixed

- Crash when trying to play an empty, shuffled queue

## [0.11.1] - 2022-09-17

### Changed

- Mouse scrolling now moves the view, not the focussed element
- Setting the track position now only works with a click, not by dragging the progress bar

### Fixed

- Back button going back twice when clicked once with the mouse
- Back button clickable area not corresponding to the text on the button
- Bug while dragging the track progress bar
- Flickering when using the Termion TUI backend
- Albums with more than 50 songs not showing all the songs when viewed in the library
- Bug that could cause items to not load until the screen is filled on bigger screens

[1.5.0]: https://github.com/hrkfdn/ncspot/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/hrkfdn/ncspot/compare/v1.3.4...v1.4.0
[1.3.4]: https://github.com/hrkfdn/ncspot/compare/v1.3.3...v1.3.4
[1.3.3]: https://github.com/hrkfdn/ncspot/compare/v1.3.2...v1.3.3
[1.3.2]: https://github.com/hrkfdn/ncspot/compare/v1.3.1...v1.3.2
[1.3.1]: https://github.com/hrkfdn/ncspot/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/hrkfdn/ncspot/compare/v1.2.2...v1.3.0
[1.2.2]: https://github.com/hrkfdn/ncspot/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/hrkfdn/ncspot/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/hrkfdn/ncspot/compare/v1.1.2...v1.2.0
[1.1.2]: https://github.com/hrkfdn/ncspot/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/hrkfdn/ncspot/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/hrkfdn/ncspot/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/hrkfdn/ncspot/compare/v0.13.4...v1.0.0
[0.13.4]: https://github.com/hrkfdn/ncspot/compare/v0.13.3...v0.13.4
[0.13.3]: https://github.com/hrkfdn/ncspot/compare/v0.13.2...v0.13.3
[0.13.2]: https://github.com/hrkfdn/ncspot/compare/v0.13.1...v0.13.2
[0.13.1]: https://github.com/hrkfdn/ncspot/compare/v0.13.0...v0.13.1
[0.13.0]: https://github.com/hrkfdn/ncspot/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/hrkfdn/ncspot/compare/v0.11.2...v0.12.0
[0.11.2]: https://github.com/hrkfdn/ncspot/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/hrkfdn/ncspot/compare/v0.11.0...v0.11.1
