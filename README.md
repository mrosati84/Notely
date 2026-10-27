# Notely

Image annotation made simple: open a screenshot, mark it up, then copy it or export it.

Notely is a small native desktop app written in Rust with [egui](https://github.com/emilk/egui).

## Features

- **Tools:** arrow, rectangle, ellipse, highlight, blur, spotlight and text
- **Styling:** color presets, plus size, blur strength and line width controls
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
| `Cmd/Ctrl+V` | Paste image from clipboard |
| `Cmd/Ctrl+C` | Copy annotated image |
| `Cmd/Ctrl+S` | Save as PNG or JPEG |
| `Cmd/Ctrl+Z` | Undo |
| `Cmd/Ctrl+Shift+Z` | Redo |
| `Delete` / `Backspace` | Delete the selected annotation, or clear all annotations if none is selected |
| `Esc` | Deselect |

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

## Project layout

```
src/
  main.rs         window setup and entry point
  app.rs          UI, input handling, clipboard and file I/O
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
