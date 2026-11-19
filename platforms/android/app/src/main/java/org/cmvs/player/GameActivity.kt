package org.cmvs.player

import android.app.AlertDialog
import android.app.NativeActivity
import android.content.Intent
import android.text.InputFilter
import android.text.InputType
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import java.io.File
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class GameActivity : NativeActivity() {
    private val cancelled = AtomicBoolean()
    private var choosingFont = false

    private val commentResult = AtomicReference<Pair<Int, String>?>()
    private var commentDialog: AlertDialog? = null

    fun beginCommentInput(id: Int, initial: String, maxChars: Int) = runOnUiThread {
        commentDialog?.dismiss()
        fun answer(value: String) = commentResult.set(id to value)
        if (isFinishing || isDestroyed) {
            answer("")
            return@runOnUiThread
        }
        val field = EditText(this)
        field.setSingleLine(true)
        field.contentDescription = "Save comment"
        field.inputType = InputType.TYPE_CLASS_TEXT
        field.imeOptions = EditorInfo.IME_ACTION_DONE
        field.setText(initial)
        field.filters = arrayOf(InputFilter.LengthFilter(maxChars))
        field.selectAll()
        val dialog = AlertDialog.Builder(this).setView(field)
            .setNegativeButton(android.R.string.cancel) { _, _ -> answer("") }
            .setOnCancelListener { answer("") }
            .setPositiveButton(android.R.string.ok) { _, _ -> answer(" ${field.text}") }
            .create()
        field.setOnEditorActionListener { _, action, _ ->
            if (action != EditorInfo.IME_ACTION_DONE) false else {
                answer(" ${field.text}")
                dialog.dismiss()
                true
            }
        }
        field.requestFocus()
        dialog.window?.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE)
        commentDialog = dialog
        dialog.setOnDismissListener { if (commentDialog === dialog) commentDialog = null }
        dialog.show()
    }

    // null means pending, empty means cancel, and an initial space marks an accepted draft.
    fun takeCommentInput(id: Int): String? = commentResult.getAndSet(null)?.takeIf { it.first == id }?.second

    fun dismissCommentInput() = runOnUiThread {
        commentDialog?.dismiss()
        commentResult.set(null)
    }

    /** Called on the UI thread by the native platform backend via chooseFont()V. */
    fun chooseFont() {
        if (choosingFont || isFinishing || isDestroyed) return
        choosingFont = true
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*")
        intent.addCategory(Intent.CATEGORY_OPENABLE)
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        try {
            startActivityForResult(intent, 1)
        } catch (_: Exception) {
            answer("cancel\n")
        }
    }

    override fun onActivityResult(request: Int, result: Int, data: Intent?) {
        super.onActivityResult(request, result, data)
        if (request != 1) return
        val uri = data?.data
        if (result != RESULT_OK || uri == null) {
            answer("cancel\n")
            return
        }
        val index = EditText(this)
        index.inputType = InputType.TYPE_CLASS_NUMBER
        index.setText("0")
        AlertDialog.Builder(this).setTitle("字体集合索引").setMessage("普通字体使用 0")
            .setView(index).setNegativeButton("取消") { _, _ -> answer("cancel\n") }
            .setOnCancelListener { answer("cancel\n") }
            .setPositiveButton("确定") { _, _ ->
                val face = try {
                    index.text.toString().toInt().also {
                        if (it !in 0..1024) throw IllegalArgumentException()
                    }
                } catch (_: Exception) {
                    answer("cancel\n")
                    return@setPositiveButton
                }
                Thread({
                    try {
                        val font = GameImport.font(this, uri, cancelled)
                        answer("${font.path}\n$face\n")
                    } catch (_: Exception) {
                        answer("cancel\n")
                    }
                }, "font-import").start()
            }.show()
    }

    private fun answer(value: String) {
        if (isDestroyed) return
        try {
            val temporary = File(filesDir, "font-result.tmp")
            Files.write(temporary.toPath(), value.toByteArray(StandardCharsets.UTF_8))
            Files.move(temporary.toPath(), File(filesDir, "font-result.cfg").toPath(), StandardCopyOption.REPLACE_EXISTING)
        } catch (_: Exception) { }
        runOnUiThread { choosingFont = false }
    }

    override fun onDestroy() {
        cancelled.set(true)
        super.onDestroy()
    }
}
