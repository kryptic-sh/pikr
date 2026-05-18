# pikr

Vim-modal picker / launcher. Rofi replacement with hjkl keys.

## Status

Shipping. Latest release on
[GitHub](https://github.com/kryptic-sh/pikr/releases). Roadmap + known gaps in
[issues](https://github.com/kryptic-sh/pikr/issues).

## Modes

- `dmenu` — read entries from stdin, print selection to stdout
- `drun` — XDG `.desktop` application launcher
- `run` — `$PATH` executable runner
- `ssh` — pick a host from `~/.ssh/config` / `~/.ssh/known_hosts`
- `emoji` — searchable Unicode emoji picker (output via stdout)
- `clipboard` — recent clipboard history
- `calc` — reactive expression evaluator

## Install

Tag releases ship `.deb` / `.rpm` / `.apk` / Homebrew formula / AUR package
alongside per-target binary tarballs.

```sh
# Debian / Ubuntu / Mint / Pop!_OS
curl -fsSLO https://github.com/kryptic-sh/pikr/releases/latest/download/pikr_*_amd64.deb
sudo dpkg -i pikr_*_amd64.deb

# Fedora / RHEL / Rocky / Alma / openSUSE
sudo dnf install https://github.com/kryptic-sh/pikr/releases/latest/download/pikr-*.x86_64.rpm

# Alpine
sudo apk add --allow-untrusted \
  https://github.com/kryptic-sh/pikr/releases/latest/download/pikr-*.apk

# Arch (AUR)
yay -S pikr-bin

# macOS / Linuxbrew
brew install kryptic-sh/tap/pikr

# Build from source
cargo install --git https://github.com/kryptic-sh/pikr pikr
```

## Usage

```sh
pikr --show drun          # launch app
pikr --show run           # run command
echo -e "a\nb\nc" | pikr --dmenu
```

## Keybindings (planned)

Normal mode:

| Key               | Action                   |
| ----------------- | ------------------------ |
| `j` / `k`         | move down / up           |
| `gg` / `G`        | top / bottom             |
| `<C-d>` / `<C-u>` | half-page down / up      |
| `/`               | start search             |
| `:`               | ex command (mode switch) |
| `<CR>`            | accept selection         |
| `<Esc>`           | cancel                   |
| `i`               | enter insert mode        |

## Config

`$XDG_CONFIG_HOME/pikr/config.toml`. No file is auto-written; in-memory defaults
are used when absent.

```toml
max_results = 256
case_sensitive = false

[theme]
bg = "#1e1e2e"
fg = "#cdd6f4"
accent = "#89b4fa"
font = "monospace"
font_size = 14.0
```

## Requirements

Requires a Wayland compositor advertising the `wlr-layer-shell-unstable-v1`
protocol (Hyprland, sway, niri, river, wayfire, …). GNOME Mutter does not
advertise the protocol — use `--no-layer-shell` for a regular Wayland window.
X11 is not supported.

## Render backend

Built on [floem](https://crates.io/crates/floem) (winit + vello), Wayland-only.

## License

MIT. See [LICENSE](LICENSE).
