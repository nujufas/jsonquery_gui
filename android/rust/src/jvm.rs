//! Calls from Rust into `MainActivity` (Kotlin), over JNI.
//!
//! Every method here has a Kotlin counterpart of the same name in
//! `MainActivity.kt`; the signatures are spelled out as JNI descriptors, so
//! changing one side means changing the other.

use android_activity::AndroidApp;
use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};

/// A handle on the running activity.
pub struct Jvm {
    vm: JavaVM,
    activity: GlobalRef,
}

impl Jvm {
    pub fn new(app: &AndroidApp) -> Option<Self> {
        // SAFETY: `android-activity` hands out the process's `JavaVM` and the
        // live `NativeActivity` object; both outlive `app`, and the activity
        // is pinned below with a global reference of our own.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
        let activity = {
            let env = vm.attach_current_thread().ok()?;
            let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
            env.new_global_ref(&activity).ok()?
        };
        Some(Self { vm, activity })
    }

    /// Run `f` on the activity with a JNI environment attached to this
    /// thread. A Java exception raised by the call is logged and cleared, so
    /// it can never be left pending across the next JNI call.
    fn call<R>(
        &self,
        f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>,
    ) -> Option<R> {
        let mut env = self.vm.attach_current_thread().ok()?;
        let result = f(&mut env, self.activity.as_obj());
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
            .map_err(|error| log::warn!("JNI call failed: {error}"))
            .ok()
    }

    /// Show the document picker; Kotlin answers through `Native.onPicked`.
    pub fn pick_open(&self, request: i32) -> bool {
        self.call(|env, activity| {
            env.call_method(activity, "pickOpen", "(I)V", &[JValue::Int(request)])
                .map(|_| ())
        })
        .is_some()
    }

    /// Show the "where to save" picker; Kotlin answers through
    /// `Native.onPicked` with a staging path to write to.
    pub fn pick_save(&self, request: i32, suggested_name: &str) -> bool {
        self.call(|env, activity| {
            let name = env.new_string(suggested_name)?;
            env.call_method(
                activity,
                "pickSave",
                "(ILjava/lang/String;)V",
                &[JValue::Int(request), JValue::Object(&name)],
            )
            .map(|_| ())
        })
        .is_some()
    }

    /// The staged file at `path` is complete: copy it to where the user chose.
    pub fn file_saved(&self, path: &str) {
        self.call(|env, activity| {
            let path = env.new_string(path)?;
            env.call_method(
                activity,
                "fileSaved",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&path)],
            )
            .map(|_| ())
        });
    }

    pub fn clipboard_text(&self) -> Option<String> {
        self.call(|env, activity| {
            let text = env
                .call_method(activity, "clipboardText", "()Ljava/lang/String;", &[])?
                .l()?;
            if text.is_null() {
                return Ok(None);
            }
            let text = JString::from(text);
            let text = env.get_string(&text)?;
            Ok(Some(String::from(text)))
        })
        .flatten()
    }

    pub fn set_clipboard_text(&self, text: &str) {
        self.call(|env, activity| {
            let text = env.new_string(text)?;
            env.call_method(
                activity,
                "setClipboardText",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&text)],
            )
            .map(|_| ())
        });
    }

    pub fn set_keyboard_visible(&self, visible: bool) {
        self.call(|env, activity| {
            env.call_method(
                activity,
                "setKeyboardVisible",
                "(Z)V",
                &[JValue::Bool(visible.into())],
            )
            .map(|_| ())
        });
    }

    pub fn set_dark_theme(&self, dark: bool) {
        self.call(|env, activity| {
            env.call_method(
                activity,
                "setDarkTheme",
                "(Z)V",
                &[JValue::Bool(dark.into())],
            )
            .map(|_| ())
        });
    }
}
