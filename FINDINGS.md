# Notely — Independent Code Review Findings

Two independent reviews of the full `notely` codebase were run in parallel and their
results merged here. Findings are deduplicated; where both reviewers found the same
issue it is credited to **Both**.

- **Scope:** all Rust sources (`src/`, `examples/`), `Cargo.toml`, `scripts/`, `macos/`.
- **Reviewers:** Claude Code (`claude -p`, model `claude`) and Codex (`codex exec`, model `gpt-5.6-sol`).
- **Method:** each model reviewed the repository read-only and emitted structured findings,
  which were then cross-checked against the source and the dependency sources
  (`egui-winit 0.31.1`, `tiny-skia 0.11.4`, `ecolor 0.31.1`) before merging.
- **Result:** 21 unique findings (3 High, 12 Medium, 6 Low).

## Summary

| # | Severity | Finding | Location | Reported by |
|---|----------|---------|----------|-------------|
| 1 | High | ✅ Done — Short arrows panic the renderer (`clamp` with inverted bounds) | `render.rs:156-163` | Codex |
| 2 | High | ✅ Done — Delete/undo during a move-drag panics or corrupts annotations | `app.rs:323-350`, `372-408` | Claude |
| 3 | High | ✅ Done — `Cmd/Ctrl+V` paste shortcut never fires | `app.rs:319`, `335-337` | Claude |
| 4 | Medium | Semi-transparent colors render darkened (premultiplied fed as straight alpha) | `app.rs:447,769`; `render.rs:111-116,194` | Claude |
| 5 | Medium | Multiple spotlights darken each other and the background repeatedly | `render.rs:14-21`, `71-96` | Both |
| 6 | Medium | JPEG export drops alpha without compositing (transparency → black) | `app.rs:246-251` | Both |
| 7 | Medium | Saving with no/unknown extension fails instead of defaulting to PNG | `app.rs:246-251` | Claude |
| 8 | Medium | No image-size validation: OOM, GPU texture limit, 0×0 → NaN | `app.rs:106-120`, `196-201`, `610-616` | Both |
| 9 | Medium | Move-drag re-renders and re-uploads the whole image every frame | `app.rs:386-399`, `191-205`; `render.rs:11-55` | Both |
| 10 | Medium | Unbounded, deep-cloning undo/redo history | `app.rs:160-177`, `341`, `376` | Both |
| 11 | Medium | No-op selection drag wipes redo history | `app.rs:372-407` | Both |
| 12 | Medium | Delete/Backspace clears everything and ignores widget focus | `app.rs:298`, `312-350` | Claude |
| 13 | Medium | Switching tools mid-drag strands drag/move state | `app.rs:287-292`, `370-416` | Claude |
| 14 | Medium | Clipboard copy may not persist on Linux (arboard dropped) | `app.rs:226` | Claude |
| 15 | Medium | DMG is signed after bundling / advertised path may not exist | `scripts/package.sh:6-14` | Both |
| 16 | Low | Degenerate rectangles/ellipses committed but invisible | `app.rs:458-467` | Codex |
| 17 | Low | Canvas padding accepts out-of-bounds annotations | `app.rs:592-636` | Codex |
| 18 | Low | Arrow shaft overshoots the arrowhead tip; preview differs | `render.rs:146-178`; `app.rs:666-667` | Claude |
| 19 | Low | `ensure_fresh` double-renders and leaves `dirty` set | `app.rs:207-213` | Claude |
| 20 | Low | Obj-C callback mutex `unwrap` can panic across FFI; multi-file drop reloads each | `macos_open.rs:49,117`; `app.rs:782-784` | Claude |
| 21 | Low | Missing `rust-version`, dead code, duplicated helpers | `Cargo.toml`; `render.rs:295,309`; `app.rs:517-532`; `examples/make_icon.rs:128-138` | Claude |

---

## High

### 1. Short arrows panic the renderer
- **Severity:** High
- **Category:** Memory safety / panics
- **Location:** `src/render.rs:156-163`
- **Reported by:** Codex
- **Description:** `draw_arrow` computes the arrowhead length with
  `let head = (width * 4.5).clamp(10.0, len * 0.6);`. `f32::clamp` **panics when
  `min > max`**. For any arrow where `len * 0.6 < 10.0` — i.e. `len < 16.67 px` — this
  is an inverted clamp and aborts the process. `commit_drag` only requires
  `length() >= 2.0` (`app.rs:451`), so a short arrow that the UI happily accepts
  (2–16.67 px) will crash the app and lose unsaved work on the very next render.
- **Recommendation:** Avoid the inverted clamp, e.g.
  `let head = (width * 4.5).max(10.0).min(len * 0.6).max(len * 0.2);` (or clamp once
  against a ceiling that is always ≥ the floor). Reject unreasonably short arrows at
  commit time and add a regression test covering 2–16 px arrows.

### 2. Delete/undo during a move-drag can panic or overwrite the wrong annotation
- **Severity:** High
- **Category:** Correctness bugs / panics
- **Location:** `src/app.rs:323-350`, `src/app.rs:372-408`, `src/app.rs:386-399`
- **Reported by:** Claude
- **Description:** A Select-mode drag stores an index into `self.annotations`
  (`MoveDrag { index, .. }`, `app.rs:380`) and then writes back with
  `self.annotations[idx] = ann` every drag frame (`app.rs:397`). `handle_keys` runs
  **before** `canvas` each frame (`update`, `app.rs:787-793`) and does not consult
  `self.moving`:
  - Pressing **Delete/Backspace** mid-drag removes the dragged annotation
    (`app.rs:343`). If it was the last element, the next `dragged()` frame indexes out
    of bounds and panics; otherwise it silently overwrites a neighbouring annotation
    with a stale copy.
  - Pressing **Cmd+Z** mid-drag pops the snapshot pushed at drag start; a second press
    shrinks the vector and can trigger the same out-of-bounds panic. The
    `if !m.moved { self.undo.pop() }` cleanup on release (`app.rs:405`) then pops an
    unrelated history entry.
- **Recommendation:** Ignore history/delete shortcuts while `self.moving.is_some()`
  (or cancel the drag first by restoring `m.original` and dropping the pushed
  snapshot). As defence in depth, use
  `if let Some(slot) = self.annotations.get_mut(idx)` instead of direct indexing.

### 3. The `Cmd/Ctrl+V` paste shortcut never fires
- **Severity:** High
- **Category:** API/UX consistency
- **Location:** `src/app.rs:319`, `src/app.rs:335-337`
- **Reported by:** Claude
- **Description:** The app detects paste with
  `i.key_pressed(egui::Key::V) && i.modifiers.command`. However, egui-winit 0.31.1
  intercepts paste/copy/cut **before** emitting a key event and `return`s early
  (`egui-winit-0.31.1/src/lib.rs:772-779`), so `Key::V` is never produced with the
  command modifier. Worse, `Event::Paste` is only pushed when the clipboard contains
  **text** (`lib.rs:773-777`), so an image-only clipboard yields no event at all. The
  image "paste" path (`paste_image`, `app.rs:145`) is therefore unreachable via the
  advertised shortcut on every platform, while the empty-canvas hint (`app.rs:603`)
  and status bar (`app.rs:586`) both advertise it.
- **Recommendation:** Watch `egui::Event::Paste(_)` for the text case, and for images
  add a native menu accelerator (e.g. `muda`) or a visible "Paste" button that calls
  `paste_image()`. Do not rely on `Key::V`.

---

## Medium

### 4. Semi-transparent annotation colors render darkened
- **Severity:** Medium
- **Category:** Correctness bugs
- **Location:** `src/app.rs:447`, `src/app.rs:769`; `src/render.rs:111-116`, `194`
- **Reported by:** Claude
- **Description:** `egui::Color32` is stored premultiplied, so `self.color.to_array()`
  returns **premultiplied** `[r, g, b, a]`. tiny-skia's `Color` is **straight
  (unpremultiplied)** alpha — `Color::premultiply()` returns `PremultipliedColor`
  (`tiny-skia-0.11.4/src/color.rs:319`), and `from_rgba8` stores the components
  verbatim — so `Color::from_rgba8(r,g,b,a)` expects straight alpha. Passing
  premultiplied values therefore premultiplies twice: any color picked with alpha
  < 255 renders darker than the swatch and the live preview. `Highlight` is affected
  too (`render.rs:194` overrides alpha to 90 while keeping the already-darkened RGB).
  (Presets are all opaque, so this only surfaces once the user edits alpha.)
- **Recommendation:** Store/convert with `self.color.to_srgba_unmultiplied()` at
  `app.rs:447`/`769`, and use the same conversion in the highlight preview
  (`app.rs:676-681`, which currently calls `self.color.r()/g()/b()`).

### 5. Multiple spotlights darken each other and the background repeatedly
- **Severity:** Medium
- **Category:** Correctness bugs
- **Location:** `src/render.rs:14-21`, `src/render.rs:71-96`
- **Reported by:** Both
- **Description:** `render` applies each `Spot` independently via `apply_spot`, which
  dims **every pixel outside its own circle** to `dim = 0.30`. With two non-overlapping
  spotlights, the bright area of each lies outside the other and is dimmed by it, and
  the shared background is dimmed twice (`0.3² = 0.09`). Using spotlight on more than
  one region is therefore broken, and the result depends on annotation order.
- **Recommendation:** Collect all `Spot` annotations and apply a single pass: per pixel
  compute the minimum normalized distance (or the maximum "lit" factor) across all
  spots, then dim once.

### 6. JPEG export drops alpha without compositing
- **Severity:** Medium
- **Category:** Correctness bugs
- **Location:** `src/app.rs:246-251`
- **Reported by:** Both
- **Description:** The JPEG branch uses
  `image::DynamicImage::ImageRgba8(img.clone()).to_rgb8()`, which discards the alpha
  channel without compositing. Fully transparent pixels in PNG screenshots (e.g. macOS
  window captures with shadows) usually carry RGB = 0, so they export as **black**
  instead of showing whatever was behind them.
- **Recommendation:** Alpha-composite the rendered image onto an explicit background
  (commonly white) before RGB conversion and JPEG encoding, or ask the user which
  background to use.

### 7. Saving with no/unknown extension fails instead of defaulting to PNG
- **Severity:** Medium
- **Category:** Error handling gaps
- **Location:** `src/app.rs:246-251`
- **Reported by:** Claude
- **Description:** The fallback branch is `img.save(&path)`, which infers the format
  from the file extension. If the user types a filename with no extension (or an
  unknown one), it fails with "the image format could not be determined" even though
  the dialog's default is `notely.png` and PNG is the intended default.
- **Recommendation:** In the fallback arm use
  `img.save_with_format(&path, image::ImageFormat::Png)` (appending `.png` if needed),
  or validate/normalize the extension before saving.

### 8. No image-size validation (OOM, GPU texture limit, 0×0 → NaN)
- **Severity:** Medium
- **Category:** Memory safety / API-UX consistency
- **Location:** `src/app.rs:106-120`, `src/app.rs:196-201`, `src/app.rs:610-616`
- **Reported by:** Both
- **Description:**
  - **Memory:** `load_bytes`/`load_path` impose no dimension or pixel-count limit.
    After decode, `to_rgba8`, the render buffer, the tiny-skia pixmap, the output
    image, the `ColorImage`, the texture upload and the cached composite all coexist,
    so a large (but decoder-legal) image can drive the process into an out-of-memory
    abort.
  - **GPU:** the full-resolution composite is uploaded as a single texture. Large
    screenshots can exceed `max_texture_side` (commonly 8192/16384); the image then
    fails to display with no user-facing error.
  - **Empty:** a 0-width or 0-height image makes `scale` infinite and `disp` NaN in
    `canvas` (`app.rs:611-616`), so every coordinate becomes NaN and hit-testing
    returns garbage.
- **Recommendation:** In `set_image`, reject zero-dimension images with a status
  message. Enforce a documented pixel/byte budget with `image::io::Limits` and report
  oversized images. For display, downscale to fit `ctx.input(|i| i.max_texture_side)`
  (keeping full resolution for export).

### 9. Move-drag re-renders and re-uploads the whole image every frame
- **Severity:** Medium
- **Category:** Resource leaks and performance
- **Location:** `src/app.rs:386-399`, `src/app.rs:191-205`; `src/render.rs:11-55`
- **Reported by:** Both
- **Description:** Every pointer frame while moving an annotation sets
  `self.dirty = true`, and the next frame `rebuild` runs synchronously: it clones the
  base image, re-runs every blur and every full-image spotlight pass, makes two more
  full-buffer copies (premultiply/unpremultiply), builds another `ColorImage`, and
  uploads a full GPU texture. On a 5K Retina screenshot this is hundreds of MB of
  memory traffic per frame, so dragging becomes very laggy; the texture also trails
  the pointer by one frame because `rebuild` runs before `handle_canvas_input`.
- **Recommendation:** During a move, render the moving annotation as an egui overlay
  (like `draw_preview`) and composite once on `drag_stopped`. At minimum, cache the
  "base + pixel effects" layer and invalidate it only when Blur/Spot annotations
  change so vector-only edits skip the blur/spotlight passes.

### 10. Unbounded, deep-cloning undo/redo history
- **Severity:** Medium
- **Category:** Resource leaks and performance
- **Location:** `src/app.rs:160-177`, `src/app.rs:341`, `src/app.rs:376`
- **Reported by:** Both
- **Description:** Every edit pushes `self.annotations.clone()`. Highlight strokes
  carry a `Vec<Pos2>` with hundreds of points and text carries strings, so memory and
  copy time grow roughly O(edits × annotation count) — effectively quadratic under
  repeated editing — and neither the undo nor the redo stack is capped.
- **Recommendation:** Store reversible commands/deltas (`Add`, `Remove(i, ann)`,
  `Replace(i, old)`) instead of whole snapshots. At minimum cap history by entry count
  or estimated memory and drop the oldest entries.

### 11. Starting a no-op selection drag wipes the redo history
- **Severity:** Medium
- **Category:** API/UX consistency
- **Location:** `src/app.rs:372-407`
- **Reported by:** Both
- **Description:** On `drag_started` the code immediately pushes an undo snapshot and
  clears `redo` (`app.rs:376-377`), before any movement occurs. egui also fires
  `drag_started` on a long press without movement. If the annotation is not moved, the
  temporary undo entry is removed (`app.rs:405`) but the cleared redo stack is not
  restored, so Undo followed by merely pressing on an annotation permanently loses
  redo history even though the document never changed.
- **Recommendation:** Do not touch history at drag start. Keep `original` in
  `MoveDrag`, and push the undo entry / clear redo only when a real movement is
  committed at drag end.

### 12. Delete/Backspace clears everything and ignores keyboard focus
- **Severity:** Medium
- **Category:** API/UX consistency
- **Location:** `src/app.rs:298`, `src/app.rs:312-350`
- **Reported by:** Claude
- **Description:** Two issues in the key handler:
  - With nothing selected, Delete/Backspace calls `clear_annotations()` and removes
    every annotation (`app.rs:347-349`). Backspace is easy to hit by accident.
  - Only the tool shortcuts are guarded by `ctx.wants_keyboard_input()`
    (`app.rs:298-311`). The block handling Delete, Backspace, Cmd+Z, Cmd+S and Copy
    runs regardless. The color picker's `DragValue`/hex fields
    (`ui.color_edit_button_srgba`, `app.rs:534`) take keyboard focus, so pressing
    Backspace in a hex/RGB field clears all annotations and Cmd+Z there undoes canvas
    history rather than the field.
- **Recommendation:** Make Delete/Backspace act only on the selected annotation and
  keep "Clear" behind its button. Return early from the whole key handler (except
  Escape) when `ctx.wants_keyboard_input()` is true.

### 13. Switching tools mid-drag strands drag/move state
- **Severity:** Medium
- **Category:** Correctness bugs
- **Location:** `src/app.rs:287-292`, `app.rs:370-416`, `app.rs:657-708`
- **Reported by:** Claude
- **Description:** The `S`/`A`/`R` shortcuts still fire during a drag. If the tool
  changes to `Select` while `self.drag` is `Some`, the Select branch returns before
  `commit_drag` (`app.rs:370-415`), so the shape drag is never committed or cleared,
  and `draw_preview` (which does not check the tool) keeps rendering the stale preview
  until the next drag. Conversely, switching away from Select during a move leaves
  `self.moving` and its undo snapshot dangling (see finding 2).
- **Recommendation:** In `set_tool`, commit or cancel any in-progress `drag`/`moving`
  (restoring `original` and reconciling history), or ignore tool shortcuts while a
  drag is active.

### 14. Clipboard copy may not persist on Linux
- **Severity:** Medium
- **Category:** Platform-specific issues
- **Location:** `src/app.rs:226`
- **Reported by:** Claude
- **Description:** `copy_to_clipboard` creates a new `arboard::Clipboard` and drops it
  immediately after `set_image`. On X11/Wayland the application itself serves the
  clipboard contents, so they can vanish when the `Clipboard` is dropped (unless a
  clipboard manager grabs them first). The app is clearly meant to be cross-platform
  (the `windows_subsystem` attribute in `main.rs`), so "Copy" can silently do nothing
  on Linux.
- **Recommendation:** Keep a long-lived `arboard::Clipboard` in `NotelyApp`, created
  lazily and reused for copy and paste. On Linux also consider
  `arboard::SetExtLinux::wait()` on a background thread.

### 15. Packaging: DMG is signed after bundling / advertised path may not exist
- **Severity:** Medium
- **Category:** Platform-specific issues
- **Location:** `scripts/package.sh:6-14`
- **Reported by:** Both
- **Description:** `cargo bundle --release` produces the `.app` (and, per Codex, the
  DMG) **before** `codesign` runs, and the script then signs only
  `target/release/bundle/osx/Notely.app` (`package.sh:8-10`). If a DMG is produced, it
  contains the unsigned app, so installs from the advertised `Notely.dmg`
  (`package.sh:14`) can fail Gatekeeper/signature checks. The reviewers disagreed on
  whether plain `cargo bundle --release` emits a DMG at all (Claude: only the `.app`
  unless a DMG format is requested; Codex: both) — in the latter case the printed DMG
  path is simply wrong. Either way the sign-after-bundle ordering is a problem, and
  `--deep` signing is discouraged by Apple.
- **Recommendation:** Build the `.app`, sign it (without `--deep`), then build the DMG
  from the signed bundle (e.g. `hdiutil create` or cargo-bundle's Apple signing
  support). Only print paths that actually exist.

---

## Low

### 16. Degenerate rectangles and ellipses are committed but not rendered
- **Severity:** Low
- **Category:** Correctness bugs
- **Location:** `src/app.rs:458-467`
- **Reported by:** Codex
- **Description:** Rect/ellipse validation uses
  `(rect.width() >= 2.0 || rect.height() >= 2.0)`. A gesture with zero width but
  sufficient height (or vice versa) is therefore committed, but tiny-skia cannot build
  a positive-area rectangle/oval for it, so the annotation is kept in history and
  hit-testing while being invisible in the export.
- **Recommendation:** Require both dimensions (`&&`), or deliberately render degenerate
  gestures as lines instead of storing an invisible shape.

### 17. Canvas padding accepts out-of-bounds annotations
- **Severity:** Low
- **Category:** API/UX consistency
- **Location:** `src/app.rs:592-636`
- **Reported by:** Codex
- **Description:** The interaction response covers the entire central panel
  (`app.rs:595-596`) while the image occupies only `img_rect`. Clicking or dragging in
  the surrounding padding produces negative or out-of-range image coordinates; such
  annotations enter history and can be selected but are partially or fully absent from
  the exported image.
- **Recommendation:** Restrict canvas interaction to `img_rect`, or clamp/reject
  annotation coordinates to the image bounds while still allowing a drag that starts
  inside the image to finish cleanly.

### 18. Arrow shaft overshoots the arrowhead tip; preview geometry differs
- **Severity:** Low
- **Category:** Correctness bugs / consistency
- **Location:** `src/render.rs:146-178`; `src/app.rs:666-667`
- **Reported by:** Claude
- **Description:** The shaft is stroked all the way to `to` with `LineCap::Round`, so
  its rounded end protrudes `width / 2` past the head's tip and the tip looks blunt at
  larger widths. The live preview uses egui's `painter.arrow`, which has a different
  head shape, so the on-canvas preview does not match the exported result.
- **Recommendation:** End the shaft at `base` (`to - dir * head`) rather than `to`, and
  draw the preview with the same geometry as the final render.

### 19. `ensure_fresh` double-renders and leaves `dirty` set
- **Severity:** Low
- **Category:** Resource leaks and performance
- **Location:** `src/app.rs:207-213`
- **Reported by:** Claude
- **Description:** `ensure_fresh` re-renders the composite but does not clear `dirty`
  or update the texture. When copy/save runs from `handle_keys` while `dirty` is true,
  the same frame's `rebuild` (`update`, `app.rs:788-790`) renders the same image a
  second time.
- **Recommendation:** Have `ensure_fresh` call `self.rebuild(ctx)` (pass `ctx` through),
  or clear `dirty` and update the texture there.

### 20. Obj-C callback mutex panic across FFI; multi-file drop reloads each file
- **Severity:** Low
- **Category:** Platform-specific issues
- **Location:** `src/macos_open.rs:49`, `src/macos_open.rs:117`; `src/app.rs:782-784`
- **Reported by:** Claude
- **Description:** `PENDING.lock().unwrap()` runs inside the Objective-C method
  `handleAppleEvent:withReplyEvent:`. A poisoned mutex would panic across the FFI
  boundary, which aborts the process. Separately, when several files are dropped onto
  the Dock icon, `take_pending` returns them all and the update loop decodes each in
  turn (`app.rs:782-784`), each replacing the previous image — wasted work on large
  files.
- **Recommendation:** Use `lock().unwrap_or_else(|e| e.into_inner())`. In the update
  loop, load only the last pending path (`take_pending().pop()`), or intentionally
  open the last one.

### 21. Missing `rust-version`, dead code, duplicated helpers
- **Severity:** Low
- **Category:** Idiomatic Rust / maintainability
- **Location:** `Cargo.toml`; `src/render.rs:295`, `309`; `src/app.rs:517-532`, `810`;
  `examples/make_icon.rs:128-138`
- **Reported by:** Claude
- **Description:**
  - `<[T]>::as_chunks_mut` requires Rust **1.88** and `usize::is_multiple_of` requires
    **1.87**, but `Cargo.toml` declares no `rust-version`, so older toolchains fail
    with confusing errors. `examples/make_icon.rs` already uses the portable
    `chunks_exact_mut(4)`.
  - `for (i, preset) in PRESETS.iter().enumerate()` followed by `let _ = i;`
    (`app.rs:517-532`) is dead code.
  - The premultiply/unpremultiply helpers are duplicated between `render.rs` and
    `examples/make_icon.rs`.
- **Recommendation:** Add `rust-version = "1.88"` (or use `chunks_exact_mut(4)` and
  `% 2 == 0`). Remove the unused `enumerate`. Optionally expose the conversion helpers
  from the crate so the example can reuse them.

---

## Review notes and verification

- **Verified against sources:** the `Cmd/Ctrl+V` behavior (egui-winit 0.31.1
  `src/lib.rs:772-779` returns before emitting `Key::V`, and only emits
  `Event::Paste` for non-empty text) and the tiny-skia premultiplication semantics
  (`Color` is straight alpha; `premultiply()` → `PremultipliedColor`) were confirmed
  by reading the vendored crate sources, not assumed.
- **Arrow panic** (finding 1) was confirmed directly: `f32::clamp` panics on inverted
  bounds, and `commit_drag` permits arrows down to 2 px.
- **Reviewer disagreement:** found only in finding 15 (whether `cargo bundle --release`
  emits a DMG by default). Both agree the DMG, if produced, is unsigned.
- **Coverage:** Codex surfaced the crash-class findings 1 and 16; Claude surfaced the
  input/history interaction bugs 2, 3, 12, 13 and the platform/color issues 4, 14, 20.
  No finding from either review was discarded as invalid; the only edits were merging
  duplicates.
