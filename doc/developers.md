# Development

## Prerequisites
- A working [Rust installation](https://www.rust-lang.org/tools/install)
- A C compiler

On Linux, you also need `pkgconf` (sometimes called `pkg-config`) and the development headers for
OpenSSL and PulseAudio (ALSA instead when building with the `alsa_backend` feature):

- Debian and derivatives:
  ```sh
  sudo apt install build-essential pkg-config libssl-dev libpulse-dev libasound2-dev
  ```
- Fedora:
  ```sh
  sudo dnf install gcc pkgconf openssl-devel pulseaudio-libs-devel alsa-lib-devel
  ```
- Arch and derivatives:
  ```sh
  # headers are included in the base packages
  sudo pacman -S base-devel pkgconf openssl libpulse alsa-lib
  ```

At runtime, MPRIS and notifications need a D-Bus session bus.

## Debugging
For simple debugging, you can pass a debug log filename:

```sh
cargo run -- -d debug.log
```

It can be difficult to debug a TUI application as it might not run well in an IDE terminal or the
terminal could be used by the text editor. It is however possible to run ttunes in its own process
and attach a debugger. On Linux this can be achieved with `gdb` or `lldb`. It is important that
[ptrace](https://www.kernel.org/doc/html/latest/admin-guide/LSM/Yama.html) is disabled for this to
work. To disable it, execute `echo 0 | sudo tee /proc/sys/kernel/yama/ptrace_scope`. This will allow
any process to inspect the memory of another process. It is automatically re-enabled after a reboot.

If ttunes has crashed you can find the latest backtrace at `$TTUNES_CACHE_DIRECTORY/backtrace.log`.
The cache directory can be shown by running `ttunes info`.

## Compiling
Compile and install the latest release with `cargo-install`:

```sh
cargo install ttunes
```

Or clone and build locally:

```sh
git clone https://github.com/iiKillish/tui-tunes
cd tui-tunes
cargo build --release
```

**You may need to manually set the audio backend on non-Linux OSes.** See [Audio
Backends](#audio-backends).

## Audio Backends
ttunes uses PulseAudio by default. Support for other backends can be enabled with the following
commands.

Rodio for macOS (CoreAudio), as the release builds use
```sh
cargo build --no-default-features --features rodio_backend,cover,share_clipboard,notify
```

PortAudio for BSDs
```sh
cargo build --no-default-features --features portaudio_backend,cover
```

Rodio for Windows (without `cover`, which only builds on Unix-like systems)
```sh
cargo build --no-default-features --features rodio_backend,share_clipboard,notify
```

## Other Features
Here are some auxiliary features you may wish to enable:

| Feature           | Default | Description                                                                                |
|-------------------|---------|--------------------------------------------------------------------------------------------|
| `cover`           | on      | Draw album art with the Kitty graphics protocol, sixel or iTerm2 images.                   |
| `mpris`           | on      | Control `ttunes` via dbus. See [Arch Wiki: MPRIS](https://wiki.archlinux.org/title/MPRIS). |
| `notify`          | on      | Send a notification to show what's playing.                                                |
| `share_clipboard` | on      | Ability to copy the URL of a song/playlist/etc. to system clipboard.                       |

Consult [Cargo.toml](/Cargo.toml) for the full list of supported features.


## Upstream
ttunes is a fork of [ncspot](https://github.com/hrkfdn/ncspot) and still merges its Spotify and
login fixes. To pull them in:

```sh
git remote add upstream https://github.com/hrkfdn/ncspot.git
git fetch upstream && git merge upstream/main
```
