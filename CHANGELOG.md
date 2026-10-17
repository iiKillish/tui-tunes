# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Startup no longer downloads every liked song and saved album again when
  they haven't changed; `:update` still refetches everything

### Fixed

- Playback stopping when Spotify closes the connection: ttunes reconnects in
  the background and the current song keeps playing
- Playback dying on `:reconnect` an hour after logging in through the
  browser, as the login's access token had expired

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
