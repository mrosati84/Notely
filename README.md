# Notely

Image annotation made simple: open a screenshot, mark it up, then copy it or export it.

Notely is a small native desktop app written in Rust with [egui](https://github.com/emilk/egui).

## Features

- **Tools:** arrow, rectangle, ellipse, highlight, blur, spotlight and text
- **Styling:** color presets and a custom color picker; per-tool stroke width, highlighter width, blur strength and text size, each with a slider and a numeric value
- **View:** zoom from 25% to 800% (100% = one image pixel per screen pixel), fit to window, pan with Space+drag, scroll or the middle mouse button
- **Interface:** follows the system light/dark appearance; icon toolbar that adapts to the window width, with every command also in the ⋯ menu
- **Input:** open a file, drag and drop, or paste from the clipboard
- **Output:** copy the annotated image to the clipboard, or save it as PNG or JPEG
- **Editing:** select, move, resize (drag a corner handle) and delete annotations, with full undo and redo
- **macOS:** drop images onto the Dock icon, or use *Open With → Notely*

Supported input formats: PNG, JPEG, GIF, BMP, WebP and TIFF.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `S` | Select tool |
| `A` | Arrow tool |
| `R` | Rectangle tool |
| `T` | Text tool |
| `Cmd/Ctrl+O` | Open image |
| `Cmd/Ctrl+V` | Paste image from clipboard |
| `Cmd/Ctrl+C` | Copy annotated image |
| `Cmd/Ctrl+S` | Save as PNG or JPEG |
| `Cmd/Ctrl+Z` | Undo |
| `Cmd/Ctrl+Shift+Z` | Redo |
| `Delete` / `Backspace` | Delete the selected annotation |
| `Esc` | Cancel the current drawing or move, else deselect |
| `+` / `-` (or `Cmd/Ctrl` + `=` / `-`) | Zoom in / out |
| `0` (or `Cmd/Ctrl+0`) | Fit to window |
| `Space` + drag | Pan |
| `Cmd/Ctrl` + scroll, or pinch | Zoom around the pointer |

The full list is also under ⋯ → *Keyboard Shortcuts* in the app.

## Building

You need a recent stable [Rust toolchain](https://rustup.rs).

```sh
cargo run --release
```

### macOS app bundle

To build `Notely.app` and a `.dmg`, install [cargo-bundle](https://github.com/burtonageo/cargo-bundle) and then run the packaging script:

```sh
cargo install cargo-bundle
./scripts/package.sh
```

The script ad-hoc signs the app. The output goes to `target/release/bundle/osx/Notely.app` and `target/release/bundle/dmg/Notely.dmg`.

### Regenerating the icon

```sh
./scripts/make_icon.sh
```

This renders `icons/icon.png` from `examples/make_icon.rs` and converts it to `icons/icon.icns`. It uses `sips` and `iconutil`, so it only runs on macOS.

### UI snapshots

To render the main UI states to PNG files without a display (useful when reviewing layout changes):

```sh
NOTELY_SNAPSHOTS=/tmp/notely-snapshots cargo test --release snapshots -- --ignored
```

## Project layout

```
src/
  main.rs         window setup and entry point
  app.rs          UI, input handling, clipboard and file I/O
  app/snapshots.rs headless UI screenshots (ignored test, see below)
  view.rs         canvas zoom/pan transform (image space <-> screen)
  theme.rs        light/dark design tokens
  widgets.rs      icon buttons, color swatches and other custom controls
  icons.rs        vector toolbar icons
  annotation.rs   tool and annotation definitions, hit testing
  render.rs       draws annotations onto the exported image
  macos_open.rs   handles files opened from the Dock or Finder (macOS)
examples/
  make_icon.rs    generates the app icon
scripts/          packaging and icon scripts
macos/            Info.plist additions (image document types)
```

## License

MIT. See [LICENSE](LICENSE).

The bundled [Inter](https://rsms.me/inter/) font (`assets/fonts/Inter-Regular.ttf`) is licensed under the SIL Open Font License 1.1. See [assets/fonts/Inter-LICENSE.txt](assets/fonts/Inter-LICENSE.txt).
