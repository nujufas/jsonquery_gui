package io.github.nujufas.jsonquery

import android.app.NativeActivity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.view.ViewGroup
import android.view.inputmethod.InputMethodManager
import android.widget.Toast
import androidx.core.content.IntentCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import java.util.concurrent.Executors

/**
 * The app's only activity: a `NativeActivity` that runs the Rust UI
 * (`libjsonquery_android.so`, see `android/rust`) and does what only the OS
 * can do for it — the document picker, the clipboard, the on-screen
 * keyboard, and receiving files and text from other apps.
 *
 * The public methods between [pickOpen] and [setDarkTheme] are called *by
 * name* from Rust over JNI (`android/rust/src/jvm.rs`); rename them there too,
 * and keep them out of R8's reach (`proguard-rules.pro`).
 */
class MainActivity : NativeActivity() {
    private lateinit var documents: Documents
    private lateinit var imeView: ImeView

    /** Copies, which can be big, run here rather than on the main thread. */
    private val worker = Executors.newSingleThreadExecutor()

    private var lastInsets: List<Int>? = null

    /** Rust's ids of the pickers the user is looking at right now. */
    private var openRequest = NO_REQUEST
    private var saveRequest = NO_REQUEST

    override fun onCreate(savedInstanceState: Bundle?) {
        // Draw behind the system bars (Android 15+ insists on it); the Rust UI
        // keeps clear of them using the insets sent below.
        WindowCompat.setDecorFitsSystemWindows(window, false)
        documents = Documents(this)
        super.onCreate(savedInstanceState)

        imeView = ImeView(this)
        addContentView(imeView, ViewGroup.LayoutParams(1, 1))

        // The Rust UI needs to know how much of each edge the system bars and the
        // keyboard cover. NativeActivity gives no dependable single moment for
        // that (the window takes its surface from the view hierarchy), so it is
        // published whenever the insets are applied, after every layout, and
        // when the window gains focus — cheap, and Rust ignores repeats.
        ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { _, insets ->
            publishInsets(insets)
            insets
        }
        window.decorView.viewTreeObserver.addOnGlobalLayoutListener { publishInsets() }
        ViewCompat.requestApplyInsets(window.decorView)

        handleIntent(intent)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) publishInsets()
    }

    /** Send Rust the current system-bar / notch / keyboard insets (pixels). */
    private fun publishInsets(insets: WindowInsetsCompat? = null) {
        val current = insets ?: ViewCompat.getRootWindowInsets(window.decorView) ?: return
        val bars = current.getInsets(
            WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
        )
        val keyboard = current.getInsets(WindowInsetsCompat.Type.ime())
        val bottom = maxOf(bars.bottom, keyboard.bottom)
        val key = listOf(bars.left, bars.top, bars.right, bottom)
        if (key == lastInsets) return
        lastInsets = key
        Log.i(TAG, "insets left=${bars.left} top=${bars.top} right=${bars.right} bottom=$bottom (keyboard ${keyboard.bottom})")
        Native.onInsets(bars.left, bars.top, bars.right, bottom)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    // ----- Called from Rust ------------------------------------------------

    /** Show the system document picker; the answer goes to [Native.onPicked]. */
    fun pickOpen(request: Int) = runOnUiThread {
        openRequest = request
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
        if (!launch(intent, REQUEST_OPEN)) Native.onPicked(request, null, null)
    }

    /** Show the system "save as" picker; the answer goes to [Native.onPicked]. */
    fun pickSave(request: Int, suggestedName: String) = runOnUiThread {
        saveRequest = request
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("application/json")
            .putExtra(Intent.EXTRA_TITLE, suggestedName)
        if (!launch(intent, REQUEST_SAVE)) Native.onPicked(request, null, null)
    }

    /** Rust has finished writing the staged file at [path]: copy it to the user's chosen place. */
    fun fileSaved(path: String) {
        worker.execute {
            val problem = documents.commit(path)
            toast(if (problem == null) getString(R.string.saved) else getString(R.string.save_failed, problem))
        }
    }

    fun clipboardText(): String? {
        val clip = getSystemService(ClipboardManager::class.java).primaryClip ?: return null
        if (clip.itemCount == 0) return null
        return clip.getItemAt(0).coerceToText(this)?.toString()
    }

    fun setClipboardText(text: String) = runOnUiThread {
        getSystemService(ClipboardManager::class.java)
            .setPrimaryClip(ClipData.newPlainText("jsonquery", text))
    }

    /** Show or hide the on-screen keyboard *for* [imeView] (see there for why). */
    fun setKeyboardVisible(visible: Boolean) = runOnUiThread {
        val manager = getSystemService(InputMethodManager::class.java)
        if (visible) {
            imeView.requestFocus()
            manager.showSoftInput(imeView, 0)
        } else {
            manager.hideSoftInputFromWindow(imeView.windowToken, 0)
        }
    }

    /** Make the status/navigation bar icons readable against the app's current theme. */
    fun setDarkTheme(dark: Boolean) = runOnUiThread {
        WindowCompat.getInsetsController(window, window.decorView).apply {
            isAppearanceLightStatusBars = !dark
            isAppearanceLightNavigationBars = !dark
        }
    }

    // ----- Results of pickers ----------------------------------------------

    @Deprecated("NativeActivity is not a ComponentActivity, so the Activity Result API is not available")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri = data?.data.takeIf { resultCode == RESULT_OK }
        when (requestCode) {
            REQUEST_OPEN -> {
                val request = openRequest
                if (uri == null) {
                    Native.onPicked(request, null, null)
                } else {
                    worker.execute { importAndThen(uri) { Native.onPicked(request, it?.path, it?.name) } }
                }
            }
            REQUEST_SAVE -> {
                val request = saveRequest
                if (uri == null) {
                    Native.onPicked(request, null, null)
                } else {
                    val staged = documents.stage(uri)
                    Native.onPicked(request, staged.path, staged.name)
                }
            }
        }
    }

    // ----- Files and text from other apps ----------------------------------

    private fun handleIntent(intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_VIEW -> intent.data?.let(::openFromOtherApp)
            Intent.ACTION_SEND -> {
                val stream = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                // Apps often share rich text (a SpannableString), which getStringExtra would drop.
                val text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
                when {
                    stream != null -> openFromOtherApp(stream)
                    text != null -> Native.onIncoming(null, text, null)
                }
            }
        }
    }

    private fun openFromOtherApp(uri: Uri) {
        worker.execute { importAndThen(uri) { file -> if (file != null) Native.onIncoming(file.path, null, file.name) } }
    }

    /** Copy [uri] into the cache, then hand the result (null if it failed) to [then]. */
    private fun importAndThen(uri: Uri, then: (CachedFile?) -> Unit) {
        val file = try {
            documents.import(uri)
        } catch (e: Exception) {
            toast(getString(R.string.open_failed, e.message ?: e.toString()))
            null
        }
        then(file)
    }

    private fun launch(intent: Intent, requestCode: Int): Boolean = try {
        @Suppress("DEPRECATION")
        startActivityForResult(intent, requestCode)
        true
    } catch (e: Exception) {
        toast(getString(R.string.no_file_picker))
        false
    }

    private fun toast(message: String) = runOnUiThread {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show()
    }

    private companion object {
        const val TAG = "jsonquery"
        const val REQUEST_OPEN = 1
        const val REQUEST_SAVE = 2
        const val NO_REQUEST = -1
    }
}
