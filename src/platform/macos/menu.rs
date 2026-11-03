//! Application menu bar.

use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::platform::{MenuCommand, PlatformEvent};

use super::app::EVENTS;
use super::objc::*;

/// A menu entry: one of Litty's own actions (sent to the event loop), an
/// AppKit selector sent along the responder chain, or a separator. Key
/// equivalents are only given where AppKit must not see the key first;
/// every other shortcut stays in the (rebindable) keymap.
enum Item {
    Litty(&'static str, MenuCommand),
    Native(&'static str, &'static CStr, &'static str, u64),
    Separator,
}

const CMD: u64 = 1 << 20;

const OPT: u64 = 1 << 19;

const CTRL: u64 = 1 << 18;

/// Actions reachable from the menu; a menu item's tag indexes this list.
const MENU_ACTIONS: [MenuCommand; 17] = MenuCommand::ALL;

/// The "Secure Keyboard Entry" item, to show its check mark.
static SECURE_ITEM: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn menu_tag(action: MenuCommand) -> isize {
    MENU_ACTIONS.iter().position(|&a| a == action).map_or(-1, |i| i as isize)
}

/// Replace winit's default menu (whose Quit calls `terminate:` directly,
/// skipping `confirm-close-surface`) with Litty's. Call after
/// [`register_reopen_handler`], which provides the event loop proxy.
pub fn install_menu() {
    extern "C" fn on_menu(_this: Id, _cmd: Id, sender: Id) {
        // SAFETY: `sender` is the NSMenuItem that was chosen.
        let tag = unsafe { msg_send!(sender, c"tag"; -> isize) };
        if let Some(&action) = usize::try_from(tag).ok().and_then(|i| MENU_ACTIONS.get(i)) {
            EVENTS.emit(PlatformEvent::Menu(action));
        }
    }

    use Item::{Litty, Native, Separator};
    let menus: [(&str, Vec<Item>); 5] = [
        (
            "Litty",
            vec![
                Native("About Litty", c"orderFrontStandardAboutPanel:", "", 0),
                Separator,
                Litty("Settings…", MenuCommand::OpenConfig),
                Litty("Check for Updates…", MenuCommand::CheckUpdate),
                Litty("Secure Keyboard Entry", MenuCommand::ToggleSecureInput),
                Separator,
                Native("Hide Litty", c"hide:", "h", CMD),
                Native("Hide Others", c"hideOtherApplications:", "h", CMD | OPT),
                Native("Show All", c"unhideAllApplications:", "", 0),
                Separator,
                Litty("Quit Litty", MenuCommand::Quit),
            ],
        ),
        (
            "Shell",
            vec![
                Litty("New Window", MenuCommand::NewWindow),
                Litty("New Tab", MenuCommand::NewTab),
                Separator,
                Litty("Split Right", MenuCommand::SplitRight),
                Litty("Split Down", MenuCommand::SplitDown),
                Separator,
                Litty("Close", MenuCommand::CloseTab),
            ],
        ),
        (
            "Edit",
            vec![
                Litty("Copy", MenuCommand::Copy),
                Litty("Paste", MenuCommand::Paste),
                Litty("Select All", MenuCommand::SelectAll),
                Separator,
                Litty("Find…", MenuCommand::Find),
                Litty("Clear Screen", MenuCommand::ClearScreen),
            ],
        ),
        (
            "View",
            vec![
                Litty("Bigger", MenuCommand::ZoomIn),
                Litty("Smaller", MenuCommand::ZoomOut),
                Litty("Actual Size", MenuCommand::ZoomReset),
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
                            MenuCommand::Quit => "q",
                            MenuCommand::OpenConfig => ",",
                            _ => "",
                        };
                        new_item(label, sel(c"littyMenu:"), key).inspect(|&it| {
                            msg_send!(it, c"setTarget:", target => Id);
                            msg_send!(it, c"setTag:", menu_tag(action) => isize);
                            if action == MenuCommand::ToggleSecureInput {
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
