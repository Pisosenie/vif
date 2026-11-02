//! The app menu on macOS gets Settings… with ⌘, where every Mac app keeps it.

use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenuItem};
use objc2_foundation::NSString;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

static PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VifMenuTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            if let Some(proxy) = PROXY.get() {
                let _ = proxy.send_event(UserEvent::Settings);
            }
        }
    }
);

/// Puts Settings… under About in the app menu winit made.
pub fn install(proxy: EventLoopProxy<UserEvent>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if PROXY.set(proxy).is_err() {
        return;
    }
    let app = NSApplication::sharedApplication(mtm);
    let Some(menu) = app.mainMenu().and_then(|m| m.itemAtIndex(0)).and_then(|i| i.submenu()) else {
        return;
    };
    let target: Retained<Target> = unsafe { msg_send![Target::alloc(mtm), init] };
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Settings…"),
            Some(sel!(openSettings:)),
            &NSString::from_str(","),
        )
    };
    unsafe { item.setTarget(Some(&target)) };
    menu.insertItem_atIndex(&item, 2);
    menu.insertItem_atIndex(&NSMenuItem::separatorItem(mtm), 3);
    // A menu item holds its target weakly; this one lives as long as the app.
    std::mem::forget(target);
}
