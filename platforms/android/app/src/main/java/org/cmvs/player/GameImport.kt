package org.cmvs.player

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean

internal object GameImport {
    class Imported(val path: String, val name: String)

    private val starts = setOf("start.ps3", "start.ps2", "start.pss")
    private val rootFiles = setOf(
        "cmvs.cfg", "cvns.cfg", "cmvs.exe", "cmvs32.exe", "cmvs64.exe", "cvns.exe",
    )

    fun game(context: Context, tree: Uri, cancelled: AtomicBoolean): Imported {
        val path = "games/import-${UUID.randomUUID()}"
        val root = File(context.filesDir, path)
        if (!root.mkdirs()) throw IOException("无法创建游戏目录")
        try {
            val id = DocumentsContract.getTreeDocumentId(tree)
            val name = name(context, DocumentsContract.buildDocumentUriUsingTree(tree, id))
            fun directory(parent: String, parentPath: String, name: String): String {
                var result: String? = null
                forEachChild(context, tree, parent, parentPath, cancelled) { childId, childName, isDirectory ->
                    if (isDirectory && childName == name) result = childId
                }
                return result ?: throw IOException("请选择包含 data/pack 的游戏根目录")
            }
            val packId = directory(directory(id, "", "data"), "data", "pack")
            var scripts = 0
            var archive = false
            forEachChild(context, tree, packId, "data/pack", cancelled) { _, childName, isDirectory ->
                if (!isDirectory && childName in starts) scripts++
                if (!isDirectory && childName == "script.cpz") archive = true
            }
            if (!archive || scripts != 1) throw IOException("请选择包含 data/pack 的游戏根目录")
            copyTree(context, tree, id, root, "", cancelled, HashSet(), 0)
            check(cancelled)
            val pack = File(root, "data/pack")
            if (!File(pack, "script.cpz").isFile ||
                starts.count { File(pack, it).isFile } != 1
            ) {
                throw IOException("请选择包含 data/pack 的游戏根目录")
            }
            return Imported(path, name)
        } catch (e: Exception) {
            remove(root)
            throw IOException(e.message, e)
        }
    }

    private fun copyTree(
        context: Context, tree: Uri, id: String, destination: File, relativePath: String,
        cancelled: AtomicBoolean, visited: MutableSet<String>, depth: Int,
    ) {
        check(cancelled)
        if (depth > 32 || visited.size >= 100000 || !visited.add(id)) {
            throw IOException("目录层级、数量异常或存在循环")
        }
        forEachChild(context, tree, id, relativePath, cancelled) { childId, childName, isDirectory ->
            val child = File(destination, childName)
            if (child.exists()) throw IOException("目录包含重复文件名")
            if (isDirectory) {
                if (!child.mkdir()) throw IOException("无法创建子目录")
                val childPath = if (relativePath.isEmpty()) childName else "$relativePath/$childName"
                copyTree(context, tree, childId, child, childPath, cancelled, visited, depth + 1)
            } else {
                copy(context, DocumentsContract.buildDocumentUriUsingTree(tree, childId), child, cancelled, Long.MAX_VALUE)
            }
        }
    }

    private fun forEachChild(
        context: Context, tree: Uri, id: String, relativePath: String, cancelled: AtomicBoolean,
        visit: (String, String, Boolean) -> Unit,
    ) {
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id)
        val columns = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
        )
        context.contentResolver.query(children, columns, null, null, null).use { cursor ->
            if (cursor == null) throw IOException("无法读取目录")
            while (cursor.moveToNext()) {
                check(cancelled)
                val childId = cursor.getString(0)?.takeIf { it.isNotEmpty() } ?: throw IOException("文件标识无效")
                val childName = cursor.getString(1)
                val mime = cursor.getString(2)
                if (childName.isNullOrEmpty() || childName == "." || childName == ".." ||
                    childName.contains('/') || childName.contains('\\')
                ) {
                    throw IOException("文件名无效")
                }
                val isDirectory = mime == DocumentsContract.Document.MIME_TYPE_DIR
                visit(childId, importName(relativePath, childName, isDirectory), isDirectory)
            }
        }
    }

    private fun importName(parent: String, name: String, isDirectory: Boolean): String {
        if (name.any { it.code >= 128 }) return name
        // Normalize only the paths opened directly by the native loader.
        val lower = name.lowercase()
        val fixed = when (parent) {
            "" -> if (isDirectory) lower == "data" || lower == "save" else lower in rootFiles
            "data" -> isDirectory && lower == "pack"
            "data/pack" -> !isDirectory && (lower in starts || lower == "script.cpz")
            "save" -> !isDirectory && lower == "system.dat"
            else -> false
        }
        return if (fixed) lower else name
    }

    fun font(context: Context, uri: Uri, cancelled: AtomicBoolean): Imported {
        val directory = File(context.filesDir, "fonts")
        if (!directory.isDirectory && !directory.mkdirs()) throw IOException("无法创建字体目录")
        val path = "fonts/font-${UUID.randomUUID()}"
        val file = File(context.filesDir, path)
        try {
            copy(context, uri, file, cancelled, 32L * 1024 * 1024)
            return Imported(path, name(context, uri))
        } catch (e: Exception) {
            remove(file)
            throw IOException(e.message, e)
        }
    }

    private fun name(context: Context, uri: Uri): String {
        try {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null).use { cursor ->
                if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) return cursor.getString(0)
            }
        } catch (_: Exception) { }
        return "已导入"
    }

    private fun copy(context: Context, source: Uri, target: File, cancelled: AtomicBoolean, limit: Long) {
        context.contentResolver.openInputStream(source).use { input ->
            FileOutputStream(target).use { output ->
                if (input == null) throw IOException("无法读取文件")
                val buffer = ByteArray(256 * 1024)
                var total = 0L
                while (true) {
                    val count = input.read(buffer)
                    if (count == -1) break
                    check(cancelled)
                    total += count
                    if (total > limit) throw IOException("字体文件不能超过 32 MiB")
                    if (target.parentFile!!.usableSpace < count + 16L * 1024 * 1024) throw IOException("存储空间不足")
                    output.write(buffer, 0, count)
                }
                output.fd.sync()
            }
        }
    }

    private fun check(cancelled: AtomicBoolean) {
        if (cancelled.get() || Thread.currentThread().isInterrupted) throw IOException("已取消")
    }

    fun remove(file: File) {
        file.listFiles()?.forEach { remove(it) }
        file.delete()
    }
}
