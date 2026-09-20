package io.github.nujufas.jsonquery

import android.content.Context
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection

/**
 * A 1x1 invisible view that exists to own an [InputConnection].
 *
 * `NativeActivity` gives the OS keyboard nothing to type into (its content
 * view has no input connection), and the window toolkit under the Rust UI
 * ignores the text a soft keyboard commits. So when a text field in the UI
 * gains focus, [MainActivity.setKeyboardVisible] focuses this view and shows
 * the keyboard *for it*; what the keyboard types arrives here and is forwarded
 * to Rust, which turns it into egui text events.
 *
 * The field is declared as a visible-password field: no suggestions, no
 * autocorrect, no auto-capitalisation — what a JSON query and JSON text want.
 */
class ImeView(context: Context) : View(context) {
    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor() = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN or
            EditorInfo.IME_FLAG_NO_EXTRACT_UI or
            EditorInfo.IME_ACTION_NONE
        return Connection(this)
    }

    /**
     * Reports everything to Rust and keeps no text of its own: the field being
     * edited is on the Rust side, so the keyboard is told there is nothing
     * before the cursor, which makes it send Backspace as a key event.
     */
    private class Connection(view: View) : BaseInputConnection(view, false) {
        override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
            Native.onSoftKeyboard(Native.COMMIT, text.toString(), 0, 0)
            return true
        }

        override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
            Native.onSoftKeyboard(Native.COMPOSE, text.toString(), 0, 0)
            return true
        }

        override fun finishComposingText(): Boolean {
            Native.onSoftKeyboard(Native.FINISH_COMPOSING, null, 0, 0)
            return true
        }

        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
            Native.onSoftKeyboard(Native.DELETE, null, beforeLength, afterLength)
            return true
        }

        override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean =
            deleteSurroundingText(beforeLength, afterLength)

        override fun sendKeyEvent(event: KeyEvent): Boolean {
            if (event.action != KeyEvent.ACTION_DOWN) return true
            when (event.keyCode) {
                KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER ->
                    Native.onSoftKeyboard(Native.KEY, null, KeyEvent.KEYCODE_ENTER, 0)
                KeyEvent.KEYCODE_DEL, KeyEvent.KEYCODE_FORWARD_DEL, KeyEvent.KEYCODE_TAB,
                KeyEvent.KEYCODE_ESCAPE, KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT,
                KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_MOVE_HOME,
                KeyEvent.KEYCODE_MOVE_END, KeyEvent.KEYCODE_PAGE_UP, KeyEvent.KEYCODE_PAGE_DOWN ->
                    Native.onSoftKeyboard(Native.KEY, null, event.keyCode, 0)
                else -> {
                    // A keyboard that types by key events rather than by committing text.
                    // (A dead key's combining accent has the high bit set, so is negative.)
                    val unicode = event.unicodeChar
                    if (unicode > 0) {
                        Native.onSoftKeyboard(Native.COMMIT, String(Character.toChars(unicode)), 0, 0)
                    }
                }
            }
            return true
        }
    }
}
