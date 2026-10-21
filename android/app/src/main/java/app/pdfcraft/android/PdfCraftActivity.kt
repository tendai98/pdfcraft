package app.pdfcraft.android

import android.app.AlertDialog
import android.app.NativeActivity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.system.Os
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import android.webkit.MimeTypeMap
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.atomic.AtomicLong

/**
 * The Android host for the Rust app (apps/pdfcraft-android). A thin forwarder to Java APIs with
 * no logic of its own (docs/android/README.md, "Kotlin helper"):
 *
 * - The Rust side calls the public methods below through JNI and polls [pollEvent] once a frame;
 *   nothing here calls into Rust, so the library exports no JNI functions.
 * - Documents chosen with the Storage Access Framework are copied into `cacheDir/saf/<n>/` and
 *   handed over as ordinary file paths, which is what the shell's pickers already deliver on the
 *   desktop. [publish] copies a file the shell wrote there back to the document it stands for.
 */
class PdfCraftActivity : NativeActivity() {
    private val events = ConcurrentLinkedQueue<String>()
    /** Staged file or folder → the document (or document tree) it stands for. */
    private val targets = ConcurrentHashMap<String, Target>()
    private val pending = ConcurrentHashMap<Int, Request>()
    private val sequence = AtomicLong(0)

    private class Target(val uri: Uri, val writable: Boolean, val tree: Boolean)

    private class Request(val id: Long, val kind: Kind, val name: String)

    private enum class Kind { OPEN, OPEN_MANY, SAVE, FOLDER }

    override fun onCreate(savedInstanceState: Bundle?) {
        // Before the Rust library loads (in super.onCreate): Rust's temp dir, settings, recovery and
        // logs follow these, as on Linux; /data/local/tmp is not writable by apps.
        try {
            Os.setenv("TMPDIR", cacheDir.absolutePath, true)
            Os.setenv("HOME", filesDir.absolutePath, true)
            Os.setenv("XDG_DATA_HOME", File(filesDir, "data").absolutePath, true)
            Os.setenv("XDG_CONFIG_HOME", File(filesDir, "config").absolutePath, true)
            Os.setenv("XDG_CACHE_HOME", cacheDir.absolutePath, true)
            Os.setenv("XDG_STATE_HOME", File(filesDir, "state").absolutePath, true)
        } catch (e: Exception) {
            events.add("error\tCouldn't set up the app folders: ${clean(e.message)}")
        }
        super.onCreate(savedInstanceState)
        drawEdgeToEdge()
        // A fresh start: what an earlier run staged is no longer tied to any document.
        File(cacheDir, "saf").deleteRecursively()
        handleViewIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleViewIntent(intent)
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        events.add("theme\t${if (isDarkMode()) 1 else 0}")
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // The listener can run before the window is laid out; by focus the insets are final.
        if (hasFocus) window.decorView.rootWindowInsets?.let { reportInsets(it) }
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        events.add("trim\t$level")
    }

    // ---- Called from Rust -------------------------------------------------------------------

    /** The next event for the Rust side, or null. One line, tab-separated (see host.rs). */
    fun pollEvent(): String? = events.poll()

    /** Choose one or more documents to open (ACTION_OPEN_DOCUMENT). */
    fun requestOpen(id: Long, mimeTypes: String, multiple: Boolean) {
        val types = mimeTypes.split(',').map { it.trim() }.filter { it.isNotEmpty() }
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = if (types.size == 1) types[0] else "*/*"
            if (types.size > 1) putExtra(Intent.EXTRA_MIME_TYPES, types.toTypedArray())
            putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple)
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
            )
        }
        launch(Request(id, if (multiple) Kind.OPEN_MANY else Kind.OPEN, ""), intent)
    }

    /** Choose where to save a new document (ACTION_CREATE_DOCUMENT). */
    fun requestSave(id: Long, name: String, mime: String) {
        val safe = displayName(name, "document.pdf")
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = mime.ifBlank { "application/octet-stream" }
            putExtra(Intent.EXTRA_TITLE, safe)
        }
        launch(Request(id, Kind.SAVE, safe), intent)
    }

    /** Choose a folder for files written one by one (ACTION_OPEN_DOCUMENT_TREE). */
    fun requestFolder(id: Long) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        }
        launch(Request(id, Kind.FOLDER, ""), intent)
    }

    /**
     * Copy the staged file at [path] to the document it stands for: "ok" when copied, "skip" when
     * the path is not a staged document (nothing to do), or "error\t<message>".
     */
    fun publish(path: String): String {
        val file = File(path)
        val direct = targets[file.absolutePath]
        return try {
            when {
                direct != null && !direct.tree -> {
                    if (!direct.writable) {
                        return "error\tThis document can't be saved back to where it came from. Use Save As to keep a copy."
                    }
                    copyOut(file, direct.uri)
                    "ok"
                }
                else -> {
                    val parent = file.parentFile?.absolutePath
                    val tree = parent?.let { targets[it] }?.takeIf { it.tree }
                        ?: return if (stagedHere(file)) "error\tThis document is no longer linked to a file. Use Save As." else "skip"
                    val folder = DocumentsContract.buildDocumentUriUsingTree(tree.uri, DocumentsContract.getTreeDocumentId(tree.uri))
                    val created = DocumentsContract.createDocument(contentResolver, folder, mimeOf(file.name), file.name)
                        ?: return "error\tThe folder refused the new file ${file.name}"
                    copyOut(file, created)
                    "ok"
                }
            }
        } catch (e: Exception) {
            "error\t${clean(e.message)}"
        }
    }

    fun clipboardText(): String? {
        val cm = getSystemService(ClipboardManager::class.java) ?: return null
        val clip = cm.primaryClip ?: return null
        if (clip.itemCount == 0) return null
        return clip.getItemAt(0).coerceToText(this)?.toString()
    }

    fun setClipboardText(text: String) {
        runOnUiThread {
            getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("PdfCraft", text))
        }
    }

    /** Dark status and navigation bar icons over a light app ([light]), light icons over a dark one. */
    @Suppress("DEPRECATION")
    fun setLightBars(light: Boolean) {
        runOnUiThread {
            if (Build.VERSION.SDK_INT >= 30) {
                val mask = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                    WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS
                window.insetsController?.setSystemBarsAppearance(if (light) mask else 0, mask)
            } else {
                val flags = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
                val decor = window.decorView
                decor.systemUiVisibility =
                    if (light) decor.systemUiVisibility or flags else decor.systemUiVisibility and flags.inv()
            }
        }
    }

    /** Back at the first screen: go to the background, keeping the app (and any unsaved work). */
    fun moveToBack() {
        runOnUiThread { moveTaskToBack(true) }
    }

    fun isDarkMode(): Boolean =
        (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES

    fun openUrl(url: String) {
        if (!(url.startsWith("https://") || url.startsWith("http://") || url.startsWith("mailto:"))) return
        runOnUiThread {
            try {
                startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
            } catch (e: Exception) {
                events.add("error\tNo app can open this link")
            }
        }
    }

    // ---- Window ------------------------------------------------------------------------------

    /**
     * Draw behind the status and navigation bars on every version (Android 15 always does), and
     * report what they, a display cutout and the soft keyboard cover as "insets" events, so the
     * Rust side keeps its controls clear of them.
     */
    @Suppress("DEPRECATION")
    private fun drawEdgeToEdge() {
        window.statusBarColor = Color.TRANSPARENT
        window.navigationBarColor = Color.TRANSPARENT
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false)
        } else {
            window.decorView.systemUiVisibility = window.decorView.systemUiVisibility or
                View.SYSTEM_UI_FLAG_LAYOUT_STABLE or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or
                View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
        }
        if (Build.VERSION.SDK_INT >= 28) {
            window.attributes = window.attributes.apply {
                layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
            }
        }
        window.decorView.setOnApplyWindowInsetsListener { view, insets ->
            reportInsets(insets)
            view.onApplyWindowInsets(insets)
        }
        window.decorView.requestApplyInsets()
    }

    @Suppress("DEPRECATION")
    private fun reportInsets(insets: WindowInsets) {
        val sides = if (Build.VERSION.SDK_INT >= 30) {
            val covered = insets.getInsets(
                WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime()
            )
            intArrayOf(covered.left, covered.top, covered.right, covered.bottom)
        } else {
            intArrayOf(
                insets.systemWindowInsetLeft, insets.systemWindowInsetTop,
                insets.systemWindowInsetRight, insets.systemWindowInsetBottom
            )
        }
        events.add("insets\t" + sides.joinToString("\t") { maxOf(it, 0).toString() })
    }

    // ---- Picker plumbing ---------------------------------------------------------------------

    private fun launch(request: Request, intent: Intent) {
        runOnUiThread {
            val code = (request.id and 0xFFFF).toInt()
            pending[code] = request
            try {
                startActivityForResult(intent, code)
            } catch (e: Exception) {
                pending.remove(code)
                events.add("error\tNo app can choose files on this device")
                events.add("picked\t${request.id}\t")
            }
        }
    }

    @Deprecated("NativeActivity is a framework Activity; this is its result callback")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val request = pending.remove(requestCode) ?: return
        if (resultCode != RESULT_OK || data == null) {
            events.add("picked\t${request.id}\t")
            return
        }
        when (request.kind) {
            Kind.OPEN, Kind.OPEN_MANY -> {
                val uris = mutableListOf<Uri>()
                data.clipData?.let { c -> for (i in 0 until c.itemCount) uris.add(c.getItemAt(i).uri) }
                if (uris.isEmpty()) data.data?.let { uris.add(it) }
                stageAll(request.id, uris, data.flags)
            }
            Kind.SAVE -> {
                val uri = data.data
                if (uri == null) {
                    events.add("picked\t${request.id}\t")
                    return
                }
                val dir = newStagingDir()
                val file = File(dir, displayName(queryName(uri) ?: request.name, request.name))
                targets[file.absolutePath] = Target(uri, writable = true, tree = false)
                events.add("picked\t${request.id}\t${file.absolutePath}")
            }
            Kind.FOLDER -> {
                val uri = data.data
                if (uri == null) {
                    events.add("picked\t${request.id}\t")
                    return
                }
                try {
                    contentResolver.takePersistableUriPermission(
                        uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                    )
                } catch (e: SecurityException) {
                    // A one-off grant still works for this session.
                }
                val dir = newStagingDir()
                targets[dir.absolutePath] = Target(uri, writable = true, tree = true)
                events.add("picked\t${request.id}\t${dir.absolutePath}")
            }
        }
    }

    /** Copy each document into the cache off the UI thread, asking first about very large ones. */
    private fun stageAll(id: Long, uris: List<Uri>, flags: Int) {
        val total = uris.sumOf { querySize(it) ?: 0L }
        if (uris.any { (querySize(it) ?: 0L) > MAX_BYTES }) {
            events.add("error\tThat file is larger than 2 GB and can't be opened")
            events.add("picked\t$id\t")
            return
        }
        val go = { Thread({ stageAndReport(id, uris, flags) }, "saf-read").start() }
        if (total > WARN_BYTES) {
            AlertDialog.Builder(this)
                .setMessage("This file is ${total / (1 shl 20)} MB. Opening it may use a lot of memory. Open it anyway?")
                .setPositiveButton("Open") { _, _ -> go() }
                .setNegativeButton("Cancel") { _, _ -> events.add("picked\t$id\t") }
                .setOnCancelListener { events.add("picked\t$id\t") }
                .show()
        } else {
            go()
        }
    }

    private fun stageAndReport(id: Long, uris: List<Uri>, flags: Int) {
        val paths = mutableListOf<String>()
        for (uri in uris) {
            try {
                paths.add(stage(uri, flags).absolutePath)
            } catch (e: Exception) {
                events.add("error\tCouldn't read ${queryName(uri) ?: "the file"}: ${clean(e.message)}")
            }
        }
        events.add("picked\t$id\t${paths.joinToString(SEP)}")
    }

    /** Copy [uri] to a new staging folder, keeping its name, and remember where it came from. */
    private fun stage(uri: Uri, flags: Int): File {
        var writable = false
        if (flags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION != 0) {
            try {
                contentResolver.takePersistableUriPermission(
                    uri, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                )
                writable = true
            } catch (e: SecurityException) {
                try {
                    contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
                } catch (e: SecurityException) {
                    // A one-off read grant is enough to copy it now.
                }
            }
        }
        if (!writable) writable = flags and Intent.FLAG_GRANT_WRITE_URI_PERMISSION != 0 && canWrite(uri)
        val file = File(newStagingDir(), displayName(queryName(uri) ?: "document.pdf", "document.pdf"))
        val input = contentResolver.openInputStream(uri) ?: throw IllegalStateException("the file couldn't be read")
        input.use { copyCapped(it, file) }
        targets[file.absolutePath] = Target(uri, writable, tree = false)
        return file
    }

    private fun handleViewIntent(intent: Intent?) {
        intent ?: return
        val uri: Uri = when (intent.action) {
            Intent.ACTION_VIEW -> intent.data
            Intent.ACTION_SEND -> @Suppress("DEPRECATION") (intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri)
            else -> null
        } ?: return
        // Handled once, also across a configuration change that recreates the activity.
        intent.action = null
        if ((querySize(uri) ?: 0L) > MAX_BYTES) {
            events.add("error\tThat file is larger than 2 GB and can't be opened")
            return
        }
        val flags = intent.flags
        Thread({
            try {
                events.add("open\t${stage(uri, flags).absolutePath}")
            } catch (e: Exception) {
                events.add("error\tCouldn't open the file: ${clean(e.message)}")
            }
        }, "saf-view").start()
    }

    // ---- Helpers -----------------------------------------------------------------------------

    private fun newStagingDir(): File {
        val dir = File(File(cacheDir, "saf"), sequence.incrementAndGet().toString())
        dir.mkdirs()
        return dir
    }

    private fun stagedHere(file: File): Boolean =
        file.absolutePath.startsWith(File(cacheDir, "saf").absolutePath + File.separator)

    private fun copyCapped(input: InputStream, file: File) {
        file.outputStream().use { out ->
            val buf = ByteArray(1 shl 16)
            var total = 0L
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                total += n
                if (total > MAX_BYTES) {
                    out.close()
                    file.delete()
                    throw IllegalStateException("the file is larger than 2 GB")
                }
                out.write(buf, 0, n)
            }
        }
    }

    /**
     * Overwrite [uri] with [file]: the shell has already validated what it wrote. "wt" truncates;
     * some providers refuse it, and only then is plain "w" used (on Android 10 "w" doesn't
     * truncate, which would leave the old file's tail after a shorter PDF).
     */
    private fun copyOut(file: File, uri: Uri) {
        val out: OutputStream = try {
            contentResolver.openOutputStream(uri, "wt")
        } catch (e: IllegalArgumentException) {
            null
        } catch (e: UnsupportedOperationException) {
            null
        } ?: contentResolver.openOutputStream(uri, "w")
            ?: throw IllegalStateException("the document couldn't be opened for writing")
        out.use { o -> file.inputStream().use { it.copyTo(o) } }
    }

    private fun canWrite(uri: Uri): Boolean = try {
        contentResolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_FLAGS), null, null, null)?.use { c ->
            c.moveToFirst() && (c.getInt(0) and DocumentsContract.Document.FLAG_SUPPORTS_WRITE) != 0
        } ?: false
    } catch (e: Exception) {
        false
    }

    private fun queryName(uri: Uri): String? = try {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst() && !c.isNull(0)) c.getString(0) else null
        }
    } catch (e: Exception) {
        null
    } ?: uri.lastPathSegment?.substringAfterLast('/')

    private fun querySize(uri: Uri): Long? = try {
        contentResolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst() && !c.isNull(0)) c.getLong(0) else null
        }
    } catch (e: Exception) {
        null
    }

    private fun mimeOf(name: String): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
            ?: "application/octet-stream"

    private companion object {
        /** Larger documents are refused, not read (docs/android/README.md). */
        const val MAX_BYTES = 2L shl 30
        /** Larger documents ask first. */
        const val WARN_BYTES = 200L shl 20
        /** Separates paths in a "picked" event. */
        const val SEP = "\u001F"

        /** A file name safe to use as one path segment. */
        fun displayName(name: String, fallback: String): String {
            val cleaned = name.filter { it >= ' ' && it != '/' && it != '\\' && it != '\u007F' }.trim().take(200)
            return if (cleaned.isEmpty() || cleaned == "." || cleaned == "..") fallback else cleaned
        }

        /** One line of text for an event: no tabs or line breaks. */
        fun clean(message: String?): String =
            (message ?: "unknown error").replace('\t', ' ').replace('\n', ' ').replace('\r', ' ').take(500)
    }
}
