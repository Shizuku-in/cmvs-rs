package org.cmvs.player

import android.app.AlertDialog
import android.app.NativeActivity
import android.content.Intent
import android.text.InputType
import android.widget.EditText
import java.io.File
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.atomic.AtomicBoolean

class GameActivity : NativeActivity() {
    private val cancelled = AtomicBoolean()
    private var choosingFont = false

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
