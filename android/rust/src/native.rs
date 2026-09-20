//! Calls from Kotlin into Rust: the `external fun`s of `Native.kt`.
//!
//! JNI finds these by name (`Java_<package>_<class>_<method>`), so the names
//! must match `io.github.nujufas.jsonquery.Native`. They run on whatever
//! thread the OS picked, so each one only files its news in [`SHARED`] and
//! wakes the UI.

// JNI's naming scheme (`Java_<package>_<class>_<method>`) is not snake case.
#![allow(non_snake_case)]

use std::path::PathBuf;

use jni::objects::{JClass, JString};
use jni::sys::jint;
use jni::JNIEnv;
use jsonquery_gui::platform::Incoming;
use jsonquery_gui::soft_keyboard::{Input, SoftKey};

use crate::shared::{Insets, SHARED};

/// A Java `String` that may be null.
fn optional_string(env: &mut JNIEnv, string: &JString) -> Option<String> {
    if string.is_null() {
        return None;
    }
    env.get_string(string).ok().map(String::from)
}

/// `Native.onPicked(requestId, path, displayName)`: the answer to
/// `pickOpen` / `pickSave`. A null `path` means the user backed out.
#[no_mangle]
pub extern "system" fn Java_io_github_nujufas_jsonquery_Native_onPicked(
    mut env: JNIEnv,
    _class: JClass,
    request: jint,
    path: JString,
    name: JString,
) {
    let path = optional_string(&mut env, &path).map(PathBuf::from);
    let name = optional_string(&mut env, &name);
    SHARED.answer_picker(request, path, name);
}

/// `Native.onIncoming(path, text, displayName)`: another app asked us to open
/// a file ("Open with") or some text ("Share").
#[no_mangle]
pub extern "system" fn Java_io_github_nujufas_jsonquery_Native_onIncoming(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
    text: JString,
    name: JString,
) {
    let name = optional_string(&mut env, &name);
    if let Some(path) = optional_string(&mut env, &path) {
        SHARED.push_incoming(Incoming::File(PathBuf::from(path)), name);
    } else if let Some(text) = optional_string(&mut env, &text) {
        SHARED.push_incoming(Incoming::Text(text), None);
    }
}

/// Kinds of `Native.onSoftKeyboard` report; the same numbers are in
/// `Native.kt`.
const COMMIT: jint = 0;
const COMPOSE: jint = 1;
const FINISH_COMPOSING: jint = 2;
const DELETE: jint = 3;
const KEY: jint = 4;

/// `Native.onSoftKeyboard(kind, text, a, b)`: what the on-screen keyboard
/// typed. `a`/`b` are the delete lengths, or (`a`) a key code.
#[no_mangle]
pub extern "system" fn Java_io_github_nujufas_jsonquery_Native_onSoftKeyboard(
    mut env: JNIEnv,
    _class: JClass,
    kind: jint,
    text: JString,
    a: jint,
    b: jint,
) {
    let text = optional_string(&mut env, &text).unwrap_or_default();
    let input = match kind {
        COMMIT => Input::Commit(text),
        COMPOSE => Input::Compose(text),
        FINISH_COMPOSING => Input::FinishComposing,
        DELETE => Input::Delete {
            before: a.max(0) as usize,
            after: b.max(0) as usize,
        },
        KEY => match soft_key(a) {
            Some(key) => Input::Key(key),
            None => return,
        },
        _ => return,
    };
    SHARED.push_keyboard(input);
}

/// The `android.view.KeyEvent` key codes a soft keyboard sends as keys.
fn soft_key(key_code: jint) -> Option<SoftKey> {
    Some(match key_code {
        66 => SoftKey::Enter,     // KEYCODE_ENTER
        67 => SoftKey::Backspace, // KEYCODE_DEL
        112 => SoftKey::Delete,   // KEYCODE_FORWARD_DEL
        61 => SoftKey::Tab,       // KEYCODE_TAB
        111 => SoftKey::Escape,   // KEYCODE_ESCAPE
        21 => SoftKey::Left,      // KEYCODE_DPAD_LEFT
        22 => SoftKey::Right,     // KEYCODE_DPAD_RIGHT
        19 => SoftKey::Up,        // KEYCODE_DPAD_UP
        20 => SoftKey::Down,      // KEYCODE_DPAD_DOWN
        122 => SoftKey::Home,     // KEYCODE_MOVE_HOME
        123 => SoftKey::End,      // KEYCODE_MOVE_END
        92 => SoftKey::PageUp,    // KEYCODE_PAGE_UP
        93 => SoftKey::PageDown,  // KEYCODE_PAGE_DOWN
        _ => return None,
    })
}

/// `Native.onInsets(left, top, right, bottom)`: how much of each edge the
/// system bars and the keyboard cover, in pixels.
#[no_mangle]
pub extern "system" fn Java_io_github_nujufas_jsonquery_Native_onInsets(
    _env: JNIEnv,
    _class: JClass,
    left: jint,
    top: jint,
    right: jint,
    bottom: jint,
) {
    SHARED.set_insets(Insets {
        left: left as f32,
        top: top as f32,
        right: right as f32,
        bottom: bottom as f32,
    });
}
