package org.cmvs.player

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.SharedPreferences
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.io.IOException
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

class LauncherActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val cancelled = AtomicBoolean()
    private lateinit var settings: SharedPreferences
    private lateinit var gameLabel: TextView
    private lateinit var fontLabel: TextView
    private lateinit var status: TextView
    private lateinit var face: EditText
    private lateinit var gameButton: Button
    private lateinit var fontButton: Button
    private lateinit var startButton: Button
    private lateinit var cancelButton: Button
    private var busy = false

    public override fun onCreate(saved: Bundle?) {
        super.onCreate(saved)
        // Reopening the app returns to the existing game instead of adding another importer.
        if (!isTaskRoot) {
            finish()
            return
        }
        settings = getSharedPreferences("launcher", MODE_PRIVATE)
        val scroll = ScrollView(this)
        val layout = LinearLayout(this)
        layout.orientation = LinearLayout.VERTICAL
        val pad = Math.round(24 * resources.displayMetrics.density)
        layout.setPadding(pad, pad, pad, pad)
        scroll.addView(layout)
        text(layout, "CMVS").textSize = 24f
        text(layout, "当前支持 Shift-JIS 游戏，支持 BGM、语音和音效。\n导入会复制游戏文件并占用额外空间，原目录不会修改。")
        gameLabel = text(layout, "")
        gameButton = button(layout, "导入游戏目录") { pick(GAME) }
        fontLabel = text(layout, "")
        fontButton = button(layout, "选择字体") { pick(FONT) }
        text(layout, "字体集合索引（普通字体填 0）")
        face = EditText(this)
        face.inputType = InputType.TYPE_CLASS_NUMBER
        face.setText(settings.getInt("face", 0).toString())
        layout.addView(face)
        startButton = button(layout, "启动") { launch() }
        cancelButton = button(layout, "取消导入") { cancelled.set(true) }
        status = text(layout, "")
        status.accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        setContentView(scroll)
        refresh()
    }

    private fun text(layout: LinearLayout, value: String): TextView {
        val view = TextView(this)
        view.text = value
        view.setPadding(0, 12, 0, 12)
        layout.addView(view)
        return view
    }

    private fun button(layout: LinearLayout, title: String, action: () -> Unit): Button {
        val button = Button(this)
        button.text = title
        button.setOnClickListener { action() }
        layout.addView(button)
        return button
    }

    private fun refresh() {
        gameLabel.text = "游戏：${settings.getString("gameName", "未导入")}"
        fontLabel.text = "字体：${settings.getString("fontName", "未选择")}"
        gameButton.isEnabled = !busy
        fontButton.isEnabled = !busy
        face.isEnabled = !busy
        startButton.isEnabled = !busy && selected("game").isDirectory && selected("font").isFile
        cancelButton.visibility = if (busy) View.VISIBLE else View.GONE
    }

    private fun selected(key: String): File = File(filesDir, settings.getString(key, "not-selected")!!)

    private fun pick(kind: Int) {
        val intent = Intent(if (kind == GAME) Intent.ACTION_OPEN_DOCUMENT_TREE else Intent.ACTION_OPEN_DOCUMENT)
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        if (kind == FONT) {
            intent.type = "*/*"
            intent.addCategory(Intent.CATEGORY_OPENABLE)
        }
        try {
            startActivityForResult(intent, kind)
        } catch (_: ActivityNotFoundException) {
            status.text = "没有可用的系统文件选择器。"
        }
    }

    override fun onActivityResult(request: Int, result: Int, data: Intent?) {
        super.onActivityResult(request, result, data)
        val uri = data?.data
        if (result != RESULT_OK || uri == null || busy) return
        busy = true
        cancelled.set(false)
        status.text = if (request == GAME) "正在导入游戏…" else "正在导入字体…"
        refresh()
        worker.execute {
            try {
                val imported = if (request == GAME) GameImport.game(this, uri, cancelled)
                    else GameImport.font(this, uri, cancelled)
                val key = if (request == GAME) "game" else "font"
                val previous = selected(key)
                // Publish only a completed import; failed imports keep the previous selection.
                val committed = settings.edit().putString(key, imported.path)
                    .putString(key + "Name", imported.name).commit()
                if (!committed) {
                    GameImport.remove(File(filesDir, imported.path))
                    throw IOException("无法保存导入配置")
                }
                val prefix = if (request == GAME) "games/" else "fonts/"
                val old = filesDir.toPath().relativize(previous.toPath()).toString().replace(File.separatorChar, '/')
                if (old.startsWith(prefix) && old != imported.path) GameImport.remove(previous)
                runOnUiThread { status.text = "导入完成。" }
            } catch (e: Exception) {
                runOnUiThread { status.text = if (cancelled.get()) "已取消，原有选择保留。" else "导入失败：${e.message}" }
            } finally {
                runOnUiThread { busy = false; refresh() }
            }
        }
    }

    private fun launch() {
        try {
            val index = face.text.toString().toInt()
            if (index !in 0..1024) throw IllegalArgumentException()
            settings.edit().putInt("face", index).apply()
            val config = "${settings.getString("game", "")}\n${settings.getString("font", "")}\n$index\n"
            val temporary = File(filesDir, "launch.tmp")
            Files.write(temporary.toPath(), config.toByteArray(StandardCharsets.UTF_8))
            Files.move(temporary.toPath(), File(filesDir, "launch.cfg").toPath(), StandardCopyOption.REPLACE_EXISTING)
            Files.deleteIfExists(File(filesDir, "launch-error.txt").toPath())
            startActivity(Intent(this, GameActivity::class.java))
        } catch (_: NumberFormatException) {
            status.text = "字体集合索引需要填写整数。"
        } catch (_: IllegalArgumentException) {
            status.text = "字体集合索引需要在 0 到 1024 之间。"
        } catch (e: Exception) {
            status.text = "无法启动：${e.message}"
        }
    }

    override fun onResume() {
        super.onResume()
        if (!::status.isInitialized) return
        val error = File(filesDir, "launch-error.txt")
        if (error.isFile) {
            try {
                status.text = String(Files.readAllBytes(error.toPath()), StandardCharsets.UTF_8)
                Files.deleteIfExists(error.toPath())
            } catch (_: Exception) {
                status.text = "游戏运行停止，请检查游戏版本和字体。"
            }
        }
    }

    @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
    override fun onBackPressed() {
        if (busy) {
            cancelled.set(true)
            return
        }
        super.onBackPressed()
    }

    override fun onDestroy() {
        cancelled.set(true)
        worker.shutdownNow()
        super.onDestroy()
    }

    private companion object {
        private const val GAME = 1
        private const val FONT = 2
    }
}
