package io.github.nujufas.jsonquery

/**
 * Calls from Kotlin into the Rust library (`android/rust/src/native.rs`).
 *
 * They run on whatever thread the OS picked and only queue news for the UI,
 * so they are cheap and safe to call from anywhere. The `@JvmStatic` makes
 * them static JNI methods, which is what the Rust side's names assume.
 */
object Native {
    init {
        // NativeActivity has already loaded the library by the time any of
        // this runs; loading it through the class loader as well is what makes
        // the `external` methods below resolvable by name.
        System.loadLibrary("jsonquery_android")
    }

    /** Kinds of [onSoftKeyboard] report; the same numbers are in native.rs. */
    const val COMMIT = 0
    const val COMPOSE = 1
    const val FINISH_COMPOSING = 2
    const val DELETE = 3
    const val KEY = 4

    /** The answer to [MainActivity.pickOpen] / [MainActivity.pickSave]; a null [path] is "cancelled". */
    @JvmStatic external fun onPicked(request: Int, path: String?, name: String?)

    /** Another app asked us to open a file ([path], "Open with") or some [text] ("Share"). */
    @JvmStatic external fun onIncoming(path: String?, text: String?, name: String?)

    /** What the on-screen keyboard typed: [kind] is one of the constants above. */
    @JvmStatic external fun onSoftKeyboard(kind: Int, text: String?, a: Int, b: Int)

    /** How much of each screen edge the system bars and the keyboard cover, in pixels. */
    @JvmStatic external fun onInsets(left: Int, top: Int, right: Int, bottom: Int)
}
