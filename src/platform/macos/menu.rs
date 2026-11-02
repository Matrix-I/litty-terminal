//! Application menu bar.

use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::keymap::Action;

use super::app::REOPEN_PROXY;
use super::objc::*;

/// A menu entry: one of Litty's own actions (sent to the event loop), an
/// AppKit selector sent along the responder chain, or a separator. Key
/// equivalents are only given where AppKit must not see the key first;
/// every other shortcut stays in the (rebindable) keymap.
enum Item {
    Litty(&'static str, Action),
    Native(&'static str, &'static CStr, &'static str, u64),
    Separator,
}

const CMD: u64 = 1 << 20;

const OPT: u64 = 1 << 19;

const CTRL: u64 = 1 << 18;

/// Actions reachable from the menu; a menu item's tag indexes this list.
const MENU_ACTIONS: [Action; 17] = [
    Action::Quit,
    Action::ToggleSecureInput,
    Action::NewWindow,
    Action::NewTab,
    Action::SplitRight,
    Action::SplitDown,
    Action::CloseTab,
    Action::Copy,
    Action::Paste,
    Action::SelectAll,
    Action::Find,
    Action::ClearScreen,
    Action::ZoomIn,
    Action::ZoomOut,
    Action::ZoomReset,
    Action::OpenConfig,
    Action::CheckUpdate,
];

/// The "Secure Keyboard Entry" item, to show its check mark.
static SECURE_ITEM: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn menu_tag(action: Action) -> isize {
    MENU_ACTIONS.iter().position(|&a| a == action).map_or(-1, |i| i as isize)
}

/// Replace winit's default menu (whose Quit calls `terminate:` directly,
/// skipping `confirm-close-surface`) with Litty's. Call after
/// [`register_reopen_handler`], which provides the event loop proxy.
pub fn install_menu() {
    extern "C" fn on_menu(_this: Id, _cmd: Id, sender: Id) {
        // SAFETY: `sender` is the NSMenuItem that was chosen.
        let tag = unsafe { msg_send!(sender, c"tag"; -> isize) };
        let action = usize::try_from(tag).ok().and_then(|i| MENU_ACTIONS.get(i));
        if let (Some(&action), Some(proxy)) = (action, REOPEN_PROXY.lock().as_ref()) {
            let _ = proxy.send_event(crate::AppEvent::Menu(action));
        }
    }

    use Item::{Litty, Native, Separator};
    let menus: [(&str, Vec<Item>); 5] = [
        (
            "Litty",
            vec![
                Native("About Litty", c"orderFrontStandardAboutPanel:", "", 0),
                Separator,
                Litty("Settings…", Action::OpenConfig),
                Litty("Check for Updates…", Action::CheckUpdate),
                Litty("Secure Keyboard Entry", Action::ToggleSecureInput),
                Separator,
                Native("Hide Litty", c"hide:", "h", CMD),
                Native("Hide Others", c"hideOtherApplications:", "h", CMD | OPT),
                Native("Show All", c"unhideAllApplications:", "", 0),
                Separator,
                Litty("Quit Litty", Action::Quit),
            ],
        ),
        (
            "Shell",
            vec![
                Litty("New Window", Action::NewWindow),
                Litty("New Tab", Action::NewTab),
                Separator,
                Litty("Split Right", Action::SplitRight),
                Litty("Split Down", Action::SplitDown),
                Separator,
                Litty("Close", Action::CloseTab),
            ],
        ),
        (
            "Edit",
            vec![
                Litty("Copy", Action::Copy),
                Litty("Paste", Action::Paste),
                Litty("Select All", Action::SelectAll),
                Separator,
                Litty("Find…", Action::Find),
                Litty("Clear Screen", Action::ClearScreen),
            ],
        ),
        (
            "View",
            vec![
                Litty("Bigger", Action::ZoomIn),
                Litty("Smaller", Action::ZoomOut),
                Litty("Actual Size", Action::ZoomReset),
                Separator,
                Native("Enter Full Screen", c"toggleFullScreen:", "f", CMD | CTRL),
            ],
        ),
        (
            "Window",
            vec![Native("Minimize", c"performMiniaturize:", "", 0), Native("Zoom", c"performZoom:", "", 0)],
        ),
    ];

    // SAFETY: standard Objective-C runtime and AppKit calls; the target
    // class and every menu object are created here and live for the
    // whole process.
    unsafe {
        let Some(app) = shared_application() else { return };
        let target = {
            let name = c"LittyMenuTarget";
            let mut cls = objc_getClass(name.as_ptr());
            if cls.is_null() {
                let Some(base) = class(c"NSObject") else { return };
                cls = objc_allocateClassPair(base, name.as_ptr(), 0);
                if cls.is_null() {
                    return;
                }
                class_addMethod(cls, sel(c"littyMenu:"), on_menu as *const (), c"v@:@".as_ptr());
                objc_registerClassPair(cls);
            }
            let Some(obj) = send_id(cls, c"alloc").and_then(|o| send_id(o, c"init")) else { return };
            obj
        };
        let (Some(menu_cls), Some(item_cls)) = (class(c"NSMenu"), class(c"NSMenuItem")) else { return };
        let new_menu = |title: &str| -> Option<Id> {
            let menu = send_id(menu_cls, c"alloc")?;
            Some(msg_send!(menu, c"initWithTitle:", ns_string(title)? => Id; -> Id))
        };
        let new_item = |title: &str, action: Id, key: &str| -> Option<Id> {
            let item = send_id(item_cls, c"alloc")?;
            let item = msg_send!(
                item,
                c"initWithTitle:action:keyEquivalent:",
                ns_string(title)? => Id,
                action => Id,
                ns_string(key)? => Id;
                -> Id
            );
            (!item.is_null()).then_some(item)
        };

        let Some(main_menu) = new_menu("") else { return };
        for (title, items) in menus {
            let Some(submenu) = new_menu(title) else { continue };
            for item in items {
                let entry = match item {
                    Separator => send_id(item_cls, c"separatorItem"),
                    Litty(label, action) => {
                        // Standard macOS shortcuts: Cmd+Q for Quit, Cmd+, for Settings.
                        let key = match action {
                            Action::Quit => "q",
                            Action::OpenConfig => ",",
                            _ => "",
                        };
                        new_item(label, sel(c"littyMenu:"), key).inspect(|&it| {
                            msg_send!(it, c"setTarget:", target => Id);
                            msg_send!(it, c"setTag:", menu_tag(action) => isize);
                            if action == Action::ToggleSecureInput {
                                SECURE_ITEM.store(it, Ordering::Relaxed);
                            }
                        })
                    }
                    Native(label, selector, key, mods) => new_item(label, sel(selector), key).inspect(|&it| {
                        if mods != 0 {
                            msg_send!(it, c"setKeyEquivalentModifierMask:", mods => u64);
                        }
                    }),
                };
                if let Some(entry) = entry {
                    msg_send!(submenu, c"addItem:", entry => Id);
                }
            }
            let Some(top) = new_item(title, std::ptr::null_mut(), "") else { continue };
            msg_send!(top, c"setSubmenu:", submenu => Id);
            msg_send!(main_menu, c"addItem:", top => Id);
            if title == "Window" {
                msg_send!(app, c"setWindowsMenu:", submenu => Id);
            }
        }
        msg_send!(app, c"setMainMenu:", main_menu => Id);
    }
}

/// Show or clear the check mark of the "Secure Keyboard Entry" menu item.
pub fn set_secure_input_menu_state(on: bool) {
    let item = SECURE_ITEM.load(Ordering::Relaxed);
    if !item.is_null() {
        // SAFETY: the item lives as long as the menu, i.e. the process.
        unsafe { msg_send!(item, c"setState:", isize::from(on) => isize) };
    }
}
