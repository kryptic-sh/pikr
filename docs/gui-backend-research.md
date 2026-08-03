# GUI Backend Research

_Status: shelved on 2026-08-04._

## Decision constraints

Any replacement for Floem must:

- use a pure-Rust GUI library;
- preserve Pikr's current GUI structure and behavior;
- preserve the existing CSS stylesheet theme contract;
- create a centered, transparent Wayland layer-shell surface with exclusive
  keyboard focus and no reserved exclusive zone;
- render only visible result rows for large modes such as emoji;
- support rich match highlighting, icons, the Vim caret, the ex bar, the status
  bar, and message-only mode;
- retain a viable regular-window path for non-Wayland platforms; and
- reach a usable cold-start state in under 500 ms on the target machine.

GTK and GNOME-based options are excluded by project direction, independently of
technical maturity.

## Current fork requirement

Pikr currently patches both Floem and Floem's winit fork:

- [`mxaddict/floem`](https://github.com/mxaddict/floem)
- [`mxaddict/winit`](https://github.com/mxaddict/winit)

The application uses fork-only layer-shell APIs in both its picker and message
window paths:

- `LayerShellConfig`
- `WindowConfig::with_layer_shell`
- `Application::new_wayland`
- `Anchor::empty()`
- `KeyboardInteractivity::Exclusive`

Current upstream Floem does not provide these APIs. Upstream `lapce/winit` and
`rust-windowing/winit` do not provide released layer-shell support either. The
active proposal,
[`rust-windowing/winit#4044`](https://github.com/rust-windowing/winit/pull/4044),
was open, conflicting, and changes-requested when checked on 2026-08-04.

A regular upstream winit window cannot be converted into an equivalent layer
surface after creation. A Wayland surface assigned the `xdg_toplevel` role
cannot also receive a layer-shell role. Removing the forks today would therefore
remove compositor-centered layer placement, layer-shell ordering, and exclusive
keyboard routing.

Fork removal also does not solve the `rustybuzz` advisory. That dependency comes
through `resvg`/`usvg`, independently of the layer-shell patches.

## Existing layout and stylesheet contract

The current layout is fixed and narrow:

```text
transparent centered layer surface
└── panel
    └── panel stack
        ├── input row
        │   ├── prompt
        │   └── query and Vim caret
        ├── virtualized result list
        │   ├── icon
        │   ├── highlighted label
        │   └── description
        ├── ex bar
        └── status bar
            ├── Vim mode
            ├── picker mode
            └── selected/total count
```

`apps/pikr/src/ui/styles/default.css` is the stable styling source. Runtime
substitution supplies theme colors, font family, and font size. The current CSS
adapter implements:

- class selectors, specificity, source order, and later-rule wins;
- `color` and `background-color`;
- pixel and percentage `width` and `height`;
- `padding` and `margin`;
- `border`, `border-color`, and `border-radius`;
- `align-items` and `justify-content`; and
- `font-family` and `font-size`.

Selection, hover, Vim mode, query caret, and match-span styling remain dynamic
application state rather than stylesheet pseudo-classes.

A replacement does not need a browser CSS engine. It must preserve the current
file, class names, cascade, placeholders, supported declarations, and dynamic
state layering. The preferred backend-neutral shape is:

```text
default.css + Theme
        ↓
hjkl_css::Stylesheet
        ↓
ComputedStyle
        ↓
backend widget styles and layout
```

## Pure-Rust alternatives

### Iced with `iced_layershell`

This is the preferred GUI-library candidate for a future prototype.

Iced supplies a Rust-native application/update model, widgets, shaped text, rich
spans, images, scrolling, keyboard and pointer input, clipboard support, and
regular desktop windows. A software renderer should be evaluated first to avoid
making GPU adapter and pipeline initialization part of cold startup.

[`iced_layershell`](https://github.com/waycrate/exwlshelleventloop) replaces the
normal desktop event loop with an SCTK-based layer-shell integration. It can
create layer surfaces and expose keyboard, pointer, clipboard, and text-input
behavior without using winit for the Wayland frontend.

Pikr would retain `hjkl_css` and map computed styles into Iced widget styles and
layout values. Iced has no native CSS engine, but this does not broaden the
current adapter's responsibility.

The standard Iced scrollable is not a direct replacement for Floem's
`virtual_stack`. Pikr's fixed row pitch makes a narrow visible-range projection
possible:

```text
first visible row = floor(scroll offset / row pitch)
visible count = ceil(viewport height / row pitch) + overscan
```

Only that result slice should produce row widgets.

Risks requiring a prototype:

- `iced_layershell` is a third-party integration rather than official Iced
  functionality;
- its custom event loop has a smaller maintenance and testing base;
- end-to-end IME behavior needs validation;
- AccessKit/AT-SPI accessibility through the custom event loop is unclear;
- FreeBSD support is not demonstrated; and
- software-renderer compatibility and cold-start time remain unmeasured.

Using it would replace deep local Floem/winit forks with a specialized upstream
integration, but it would not eliminate all layer-shell dependency risk.

### COSMIC/libcosmic

COSMIC demonstrates that a production launcher can be built on a Rust-native
Iced-derived stack with layer-shell support. It is not preferred for Pikr
because it carries COSMIC-specific application and theme conventions, uses its
own Iced ecosystem, and provides little advantage over testing Iced directly.
Preserving Pikr's stylesheet would require bypassing much of COSMIC's design
system.

### Slint

Slint offers a mature declarative model and software rendering, but its normal
desktop backends do not provide layer-shell support. A Pikr migration would need
a custom Slint platform/window backend and a parallel adapter from the existing
stylesheet into Slint properties. That is too much backend ownership for the
expected benefit.

### egui

egui can render fixed visible row ranges and mixed-format text, but no mature,
complete layer-shell integration was found. Available adapters were experimental
and lacked important platform behavior. Its immediate-mode style model would
also leave the complete CSS mapping with Pikr.

### Direct SCTK application frontend

A narrow Pikr-specific frontend is the fallback if Iced's layer integration
fails a hard requirement. It would use existing Rust components rather than
create a general toolkit:

- `smithay-client-toolkit` and Wayland protocol crates for surfaces and input;
- `wl_shm` for software buffers;
- `tiny-skia` for shapes, clipping, and image composition;
- `cosmic-text` for shaping, fallback fonts, metrics, and glyph rendering;
- the existing `resvg` icon path;
- AccessKit for accessibility; and
- the existing `hjkl_css` parser and a backend-neutral computed style model.

Pikr's fixed screen can be laid out as explicit rectangles without a generic
widget tree. This gives maximum startup control and avoids both winit and a
specialized GUI event loop.

The cost is permanent ownership of:

- layer-surface, frame callback, SHM buffer, and damage lifecycles;
- logical, physical, integer, and fractional scaling;
- XKB keymaps, modifiers, repeat, compose, and text-input-v3 IME behavior;
- clipboard MIME offers and asynchronous data pipes;
- pointer input, scrolling, hit testing, and cursor behavior;
- glyph and image caches;
- accessibility roles, focus, actions, bounds, and announcements; and
- compositor-specific compatibility.

This is reasonable only as a fixed Pikr frontend, not as a reusable GUI library.

## Rejected directions

### General GUI toolkit from scratch

A general toolkit would add arbitrary widget trees, layout negotiation, focus
traversal, editable text, selection, undo, clipboard and drag-and-drop, popups,
event propagation, animation, theme inheritance, generic virtualization,
accessibility semantics, renderer abstraction, multiple platform backends, and
public API stability.

Pikr has one fixed interface and cannot justify or amortize that scope. Building
a general toolkit is rejected.

### Browser/WebView frontend

A browser would provide direct CSS fidelity, but layer-shell integration would
still need a native host and browser-engine startup threatens the cold-launch
target. Runtime and packaging costs are disproportionate for a transient
launcher.

### GPU-first custom rendering

A Vello/wgpu custom frontend would still require all shell, input, layout, IME,
clipboard, and accessibility work while adding GPU initialization to the startup
path. A software-first renderer is a better fit for this fixed, small surface.

## Future prototype plan

When this work resumes, build a disposable Iced spike before changing Pikr's
production frontend. It must demonstrate:

1. an unanchored, centered, transparent layer surface;
2. exclusive keyboard interactivity and reliable first-key delivery;
3. the current panel, input, result, ex-bar, status-bar, and message layouts;
4. the existing stylesheet contract through a backend-neutral style layer;
5. visible-row-only rendering for large result sets;
6. icons, rich match spans, selected/visual states, and the Vim caret;
7. IME preedit and commit behavior;
8. accessibility delivery to AT-SPI;
9. correct behavior under Sway, Hyprland, niri, and KWin;
10. a verified FreeBSD build; and
11. usable cold startup under 500 ms on the target graphical session.

If the spike passes, migrate to Iced incrementally after extracting plain picker
state, Pikr-owned input types, and backend-neutral computed styles. If it fails
because of intrinsic `iced_layershell` limitations, prototype the narrow SCTK
software frontend. Do not proceed to another toolkit adapter without identifying
a concrete advantage over those two paths.

## References

- [Floem](https://github.com/lapce/floem)
- [winit layer-shell proposal](https://github.com/rust-windowing/winit/pull/4044)
- [Iced](https://github.com/iced-rs/iced)
- [`iced_layershell`](https://github.com/waycrate/exwlshelleventloop)
- [Smithay Client Toolkit](https://github.com/Smithay/client-toolkit)
- [wlr layer-shell protocol](https://wayland.app/protocols/wlr-layer-shell-unstable-v1)
- [Wayland text-input-v3 protocol](https://wayland.app/protocols/text-input-unstable-v3)
- [`tiny-skia`](https://github.com/linebender/tiny-skia)
- [`cosmic-text`](https://github.com/pop-os/cosmic-text)
- [AccessKit](https://github.com/AccessKit/accesskit)
