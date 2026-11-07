// SPDX-License-Identifier: MIT OR Apache-2.0

//! [`install`] watches for `NSApplicationWillFinishLaunchingNotification`, which AppKit posts
//! inside `-[NSApplication finishLaunching]`: after winit set its delegate (in `EventLoop::new`)
//! and before AppKit delivers the documents the app was launched with. At that moment it adds
//! `application:openURLs:` to the delegate's class, so the launch's own documents (the event
//! comes before the first frame) and later ones (the app already running) both reach the handler.
//!
//! The only module in the workspace besides redrob-photo-tablet's that allows `unsafe`: plain
//! Objective-C messages and one `class_addMethod`, each with a `SAFETY:` comment.

#![allow(unsafe_code)]

use std::ffi::{CStr, OsStr, c_char};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Imp, Sel};
use objc2::{class, msg_send, sel};

use crate::Handler;

static HANDLER: OnceLock<Handler> = OnceLock::new();

type OpenUrls = extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject);

pub fn install(handler: Handler) {
    if HANDLER.set(handler).is_err() {
        return; // installed already
    }
    let Some(mut builder) = ClassBuilder::new(c"RedrobOpenDocumentsObserver", class!(NSObject))
    else {
        return;
    };
    // SAFETY: the function's signature is the selector's: one object argument, no return value.
    unsafe {
        builder.add_method(
            sel!(applicationWillFinishLaunching:),
            will_finish_launching as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
    }
    let observer_class = builder.register();
    // SAFETY: Foundation messages with the argument and return types they declare.
    unsafe {
        let observer: Retained<AnyObject> = msg_send![observer_class, new];
        let center: Option<Retained<AnyObject>> =
            msg_send![class!(NSNotificationCenter), defaultCenter];
        let name: Option<Retained<AnyObject>> = msg_send![
            class!(NSString),
            stringWithUTF8String: c"NSApplicationWillFinishLaunchingNotification".as_ptr()
        ];
        let (Some(center), Some(name)) = (center, name) else {
            return;
        };
        let _: () = msg_send![
            &*center,
            addObserver: &*observer,
            selector: sel!(applicationWillFinishLaunching:),
            name: &*name,
            object: ptr::null_mut::<AnyObject>()
        ];
        // The center does not retain its observers; this one lives as long as the process.
        std::mem::forget(observer);
    }
}

/// Adds `application:openURLs:` to the application delegate's class.
extern "C-unwind" fn will_finish_launching(
    _this: *mut AnyObject,
    _cmd: Sel,
    _notification: *mut AnyObject,
) {
    let selector = sel!(application:openURLs:);
    // SAFETY: AppKit messages, on the main thread (AppKit launches there and posts this
    // notification on the thread that launches).
    let (app, delegate) = unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![app, delegate];
        (app, delegate)
    };
    // SAFETY: a non-null delegate is a live object: winit holds a strong reference to it.
    let Some(delegate_ref) = (unsafe { delegate.as_ref() }) else {
        return;
    };
    let delegate_class: &AnyClass = delegate_ref.class();
    if delegate_class.responds_to(selector) {
        return; // a delegate that handles it itself
    }
    let method: OpenUrls = open_urls;
    // SAFETY: an `Imp` is called with the signature the type encoding below declares, "v@:@@":
    // (self, _cmd, NSApplication *, NSArray<NSURL *> *) -> void, which is `OpenUrls`.
    let imp = unsafe { std::mem::transmute::<OpenUrls, Imp>(method) };
    // SAFETY: adding a method to a registered class is allowed at any time; the class pointer comes
    // from a live object, and the encoding string is NUL-terminated.
    let added = unsafe {
        objc2::ffi::class_addMethod(
            ptr::from_ref(delegate_class).cast_mut(),
            selector,
            imp,
            c"v@:@@".as_ptr(),
        )
    };
    if !added.as_bool() {
        return;
    }
    // NSApplication may have noted what the delegate answers when the delegate was set; setting it
    // again makes it look again. winit keeps its own strong reference to the delegate meanwhile.
    // SAFETY: `setDelegate:` takes an object or nil.
    unsafe {
        let _: () = msg_send![app, setDelegate: ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![app, setDelegate: delegate];
    }
}

/// `-[NSApplicationDelegate application:openURLs:]`: hands every file URL's path to the handler.
extern "C-unwind" fn open_urls(
    _this: &AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
    urls: *mut AnyObject,
) {
    let Some(handler) = HANDLER.get() else {
        return;
    };
    if urls.is_null() {
        return;
    }
    let mut paths = Vec::new();
    // SAFETY: `urls` is the NSArray<NSURL *> AppKit passes; each message is one NSArray, NSURL or
    // NSString declares, and the UTF-8 buffer is copied out before the autorelease pool drains.
    unsafe {
        let count: usize = msg_send![urls, count];
        for i in 0..count {
            let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
            if url.is_null() {
                continue;
            }
            let is_file: Bool = msg_send![url, isFileURL];
            if !is_file.as_bool() {
                continue;
            }
            let path: *mut AnyObject = msg_send![url, path];
            if path.is_null() {
                continue;
            }
            let utf8: *const c_char = msg_send![path, UTF8String];
            if utf8.is_null() {
                continue;
            }
            let bytes = CStr::from_ptr(utf8).to_bytes();
            paths.push(PathBuf::from(OsStr::from_bytes(bytes)));
        }
    }
    if !paths.is_empty() {
        handler(paths);
    }
}
