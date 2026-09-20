package io.github.nujufas.jsonquery

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

/** A file in the app cache and what the user knows it as. */
class CachedFile(val path: String, val name: String)

/**
 * Moves documents between the places Android keeps them (`content://` URIs
 * from the document picker, other apps, the share sheet) and plain files in
 * the app cache, which is all the Rust side knows how to open and write.
 *
 * Opening copies the document in; saving is the reverse — Rust writes a
 * *staged* file, and [commit] copies it out to the URI the user picked.
 * Every copy gets a directory of its own, so a document that is already open
 * (and memory-mapped) is never overwritten by opening another with the same name.
 */
class Documents(private val context: Context) {
    private val inbox = File(context.cacheDir, "inbox")
    private val outbox = File(context.cacheDir, "outbox")
    private val counter = AtomicInteger()
    private val staged = ConcurrentHashMap<String, Uri>()

    init {
        // Leftovers of an earlier run of the app. Only once per process: a
        // second activity in the same process must not delete files that are
        // still open.
        if (cleaned.compareAndSet(false, true)) {
            inbox.deleteRecursively()
            outbox.deleteRecursively()
        }
    }

    /** Copy the document at [uri] into the cache. Blocking: call off the main thread. */
    fun import(uri: Uri): CachedFile {
        val name = displayName(uri) ?: "document.json"
        val file = newFile(inbox, name)
        val input = context.contentResolver.openInputStream(uri)
            ?: throw java.io.FileNotFoundException("cannot open $uri")
        input.use { source -> file.outputStream().use { source.copyTo(it, BUFFER_SIZE) } }
        return CachedFile(file.absolutePath, name)
    }

    /** A file for Rust to write, that [commit] will copy to [uri] afterwards. */
    fun stage(uri: Uri): CachedFile {
        val name = displayName(uri) ?: "results.json"
        val file = newFile(outbox, name)
        staged[file.absolutePath] = uri
        return CachedFile(file.absolutePath, name)
    }

    /** Copy the finished staged file at [path] to where the user chose; null on success, else why not. */
    fun commit(path: String): String? {
        val uri = staged.remove(path) ?: return "unknown file $path"
        val file = File(path)
        return try {
            val output = context.contentResolver.openOutputStream(uri, "wt")
                ?: throw java.io.FileNotFoundException("cannot write $uri")
            output.use { sink -> file.inputStream().use { it.copyTo(sink, BUFFER_SIZE) } }
            file.delete()
            null
        } catch (e: Exception) {
            e.message ?: e.toString()
        }
    }

    /** What the picker calls the document, or null if it does not say. */
    fun displayName(uri: Uri): String? {
        val fromProvider = try {
            context.contentResolver
                .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                ?.use { if (it.moveToFirst()) it.getString(0) else null }
        } catch (e: Exception) {
            null
        }
        return fromProvider ?: uri.lastPathSegment?.substringAfterLast('/')
    }

    private fun newFile(root: File, name: String): File {
        val dir = File(root, counter.incrementAndGet().toString()).apply { mkdirs() }
        return File(dir, sanitize(name))
    }

    private fun sanitize(name: String): String =
        name.replace(Regex("[\\\\/:*?\"<>|\\u0000-\\u001f]"), "_").take(120).ifBlank { "document.json" }

    private companion object {
        const val BUFFER_SIZE = 1 shl 20
        val cleaned = AtomicBoolean(false)
    }
}
