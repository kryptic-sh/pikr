# pikr

Vim-modal picker / launcher. Rofi replacement with hjkl keys.

## Status

v0.1 scaffold. Not yet usable. See
[issues](https://github.com/kryptic-sh/pikr/issues) for milestones.

## Modes

- `dmenu` — read entries from stdin, print selection to stdout
- `drun` — XDG `.desktop` application launcher
- `run` — `$PATH` executable runner

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

## Render backend

Built on [floem](https://crates.io/crates/floem) (winit + vello). Wayland
layer-shell support is tracked in Epic 4 — landing via a winit fork at
`mxaddict/winit` which will be upstreamed once proven.

## License

MIT. See [LICENSE](LICENSE).
