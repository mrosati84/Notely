//! macOS: open image files that are dropped onto the app's Dock icon.
//!
//! Dropping a file onto a Dock icon (or "Open With") makes macOS send a
//! `kAEOpenDocuments` (`odoc`) Apple Event to the app. Winit installs its own
//! `NSApplicationDelegate`, so instead of replacing it we register a dedicated
//! handler for that event through `NSAppleEventManager`, which is the standard
//! way to handle open-document events without owning the app delegate.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_foundation::{
    MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSNotification,
    NSNotificationCenter, NSObject, NSObjectProtocol, NSString,
};

const K_CORE_EVENT_CLASS: u32 = 0x6165_7674; // 'aevt'
const K_AE_OPEN_DOCUMENTS: u32 = 0x6f64_6f63; // 'odoc'
const K_AE_DIRECT_OBJECT: u32 = 0x2d2d_2d2d; // '----'
const K_TYPE_FILE_URL: u32 = 0x6675_726c; // 'furl'

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();
static INSTALLED: OnceLock<()> = OnceLock::new();

declare_class!(
    struct OpenHandler;

    unsafe impl ClassType for OpenHandler {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "NotelyOpenHandler";
    }

    impl DeclaredClass for OpenHandler {}

    unsafe impl NSObjectProtocol for OpenHandler {}

    unsafe impl OpenHandler {
        #[method(handleAppleEvent:withReplyEvent:)]
        fn handle_event(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let paths = unsafe { extract_paths(event) };
            if paths.is_empty() {
                return;
            }
            PENDING.lock().unwrap().extend(paths);
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
        }

        #[method(applicationWillFinishLaunching:)]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            // AppKit has just installed its default Apple Event handlers;
            // replace the `odoc` one before the launch event is dispatched.
            unsafe { register_odoc(self) };
        }
    }
);

unsafe fn register_odoc(handler: &OpenHandler) {
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let selector = Sel::register("handleAppleEvent:withReplyEvent:");
    let _: () = msg_send![
        &manager,
        setEventHandler: handler,
        andSelector: selector,
        forEventClass: K_CORE_EVENT_CLASS,
        andEventID: K_AE_OPEN_DOCUMENTS
    ];
}

/// Installs the Apple Event handler.
///
/// Call this from `main` *before* the event loop starts. Registering the
/// handler right away is not enough: `-[NSApplication finishLaunching]`
/// installs AppKit's default handlers and overwrites ours, and the default
/// `odoc` handler forwards to `NSDocumentController`, which rejects the file
/// with a "cannot open files in the … format" alert. So we (re)register from
/// `NSApplicationWillFinishLaunchingNotification`, which fires after AppKit's
/// handlers are in place but before the launch `odoc` event is delivered.
/// Safe to call more than once.
pub fn install() {
    if INSTALLED.set(()).is_err() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };

    unsafe {
        let handler: Retained<OpenHandler> =
            msg_send_id![super(mtm.alloc::<OpenHandler>().set_ivars(())), init];
        register_odoc(&handler);
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &handler,
            Sel::register("applicationWillFinishLaunching:"),
            Some(&NSString::from_str("NSApplicationWillFinishLaunchingNotification")),
            None,
        );
        // NSAppleEventManager and NSNotificationCenter keep weak references;
        // leak so the handler lives for the whole process.
        std::mem::forget(handler);
    }
}

/// Provides the egui context used to wake the UI when a document arrives.
pub fn set_context(ctx: egui::Context) {
    let _ = CTX.set(ctx);
}

/// Paths received since the last call.
pub fn take_pending() -> Vec<PathBuf> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

unsafe fn extract_paths(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // The direct object holds the file(s): a list of aliases/file-URLs for a
    // Dock drop, or a single alias/file-URL when opened through AppleScript.
    let direct: *mut NSAppleEventDescriptor =
        msg_send![event, paramDescriptorForKeyword: K_AE_DIRECT_OBJECT];
    if direct.is_null() {
        return paths;
    }
    let direct = &*direct;

    let count = direct.numberOfItems();
    if count > 0 {
        // `descriptorAtIndex:` is 1-based.
        for i in 1..=count {
            if let Some(item) = direct.descriptorAtIndex(i) {
                if let Some(path) = item_to_path(&item) {
                    paths.push(path);
                }
            }
        }
    } else if let Some(path) = item_to_path(direct) {
        paths.push(path);
    }

    paths
}

unsafe fn item_to_path(item: &NSAppleEventDescriptor) -> Option<PathBuf> {
    // Bookmark/alias/file-URL descriptors are coerced to a file URL.
    let url = item.fileURLValue().or_else(|| {
        let coerced: *mut NSAppleEventDescriptor =
            msg_send![item, coerceToDescriptorType: K_TYPE_FILE_URL];
        if coerced.is_null() {
            None
        } else {
            (&*coerced).fileURLValue()
        }
    });
    let path = url?.path()?;
    Some(PathBuf::from(path.to_string()))
}
