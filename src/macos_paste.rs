//! macOS: make ⌘V paste an image.
//!
//! `egui-winit` translates ⌘V into `egui::Event::Paste` only when the clipboard
//! holds text, and otherwise swallows the key event, so the app can never see an
//! image paste. We install a local `NSEvent` monitor that intercepts ⌘V before
//! winit does. When no widget is taking keyboard input (the user is not typing
//! into a text field) we consume the event and ask the app to paste an image;
//! otherwise we let it through so normal text paste keeps working.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use block2::RcBlock;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};

static PASTE_REQUESTED: AtomicBool = AtomicBool::new(false);
static TEXT_INPUT_ACTIVE: AtomicBool = AtomicBool::new(false);
static CTX: OnceLock<egui::Context> = OnceLock::new();
static INSTALLED: OnceLock<()> = OnceLock::new();

/// Records whether a widget currently wants keyboard input, so that ⌘V is left
/// alone while the user is editing text.
pub fn set_text_input_active(active: bool) {
    TEXT_INPUT_ACTIVE.store(active, Ordering::Relaxed);
}

/// Whether a ⌘V image paste has been requested since the last call.
pub fn take_requested() -> bool {
    PASTE_REQUESTED.swap(false, Ordering::Relaxed)
}

/// Installs the event monitor. Call once, on the main thread, after the egui
/// context exists.
pub fn install(ctx: egui::Context) {
    if INSTALLED.set(()).is_err() {
        return;
    }
    let _ = CTX.set(ctx);

    let handler = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        let event_ref = unsafe { event.as_ref() };
        if is_command_v(event_ref) && !TEXT_INPUT_ACTIVE.load(Ordering::Relaxed) {
            PASTE_REQUESTED.store(true, Ordering::Relaxed);
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
            // Returning null consumes the event.
            std::ptr::null_mut()
        } else {
            // Returning the event lets AppKit dispatch it normally.
            event.as_ptr()
        }
    });

    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
    };

    // The monitor is removed when the returned token is deallocated and the
    // block is owned by the monitor, so keep both alive for the whole process.
    std::mem::forget(monitor);
    std::mem::forget(handler);
}

fn is_command_v(event: &NSEvent) -> bool {
    unsafe {
        if !event.modifierFlags().contains(NSEventModifierFlags::NSEventModifierFlagCommand) {
            return false;
        }
        // `charactersIgnoringModifiers` still applies Shift, so a plain ⌘V is
        // "v" while ⌘⇧V is "V"; requiring the lowercase form keeps the monitor
        // from stealing other shortcuts.
        matches!(event.charactersIgnoringModifiers(), Some(chars) if chars.to_string() == "v")
    }
}
