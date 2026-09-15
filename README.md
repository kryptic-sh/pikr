# pikr

Vim-modal picker / launcher. Rofi replacement with hjkl keys.

[![CI](https://github.com/kryptic-sh/pikr/actions/workflows/ci.yml/badge.svg)](https://github.com/kryptic-sh/pikr/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/kryptic-sh/pikr)](https://github.com/kryptic-sh/pikr/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

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

### Custom accept keys (dmenu)

`--kb-custom KEY` adds an alternate key that accepts the highlighted row. The
row is printed exactly as with Enter, but pikr exits with **10** for the first
binding, **11** for the second, and so on (up to 19 bindings, like rofi's
`-kb-custom-N`), so a script can tell which key was pressed:

```sh
choice=$(printf 'home\nwork\n' | pikr --dmenu --kb-custom Shift+Delete)
case $? in
  0)  connect "$choice" ;;
  10) forget  "$choice" ;;
esac
```

`KEY` is a chord of optional modifiers (`Ctrl`, `Shift`, `Alt`, `Super`) and a
key: a single character, `Delete`, `Backspace`, `Insert`, `Return`, `Tab`,
`Escape`, `Space`, `Left`/`Right`/`Up`/`Down`, `Home`, `End`, `PageUp`,
`PageDown`, or `F1`–`F12`. Modifiers match exactly (`Delete` does not fire on
`Shift+Delete`), and bindings take precedence over the built-in keymap. With no
matching row the typed query is printed, as with Enter. Custom accepts don't
update frecency or query history.

Append `=PROMPT` to ask first. The key then opens a confirm card showing
`PROMPT` at the right edge of the highlighted row: Enter accepts with the
binding's exit code, Esc or Left dismisses, and other keys are ignored while it
is open.

```sh
choice=$(nmcli -g ssid dev wifi list | pikr --dmenu --kb-custom 'Right=Forget?')
[ $? -eq 10 ] && nmcli connection delete "$choice"
```

### Loading state (dmenu)

By default pikr reads all of stdin before opening. For a slow producer,
`--loading TEXT` opens the window immediately and shows `TEXT` centred where the
list will appear; the rows fill in once stdin closes. Typing works meanwhile,
but accepting is disabled until the rows arrive.

```sh
slow_scan | pikr --dmenu --loading 'Scanning…'
```

`Left`, `Right`, `Home` and `End` bindings fire only when the query caret can't
move that way (`Right` and `End` at the end of the query, `Left` and `Home` at
its start), so they don't take over caret movement while editing.

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

`$XDG_CONFIG_HOME/pikr/config.toml` (`%APPDATA%\pikr\config.toml` on Windows).
No file is auto-written; in-memory defaults are used when absent.

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

## Architecture

Built on [floem](https://crates.io/crates/floem) (winit + vello), Wayland-only.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) (if exists) or open an issue / PR.

## License

MIT. See [LICENSE](LICENSE).
