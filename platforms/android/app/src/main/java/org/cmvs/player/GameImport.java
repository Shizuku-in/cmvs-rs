package org.cmvs.player;

import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.HashSet;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicBoolean;

final class GameImport {
    static final class Imported {
        final String path, name;
        Imported(String path, String name) { this.path = path; this.name = name; }
    }

    static Imported game(Context context, Uri tree, AtomicBoolean cancelled) throws IOException {
        String path = "games/import-" + UUID.randomUUID();
        File root = new File(context.getFilesDir(), path);
        if (!root.mkdirs()) throw new IOException("无法创建游戏目录");
        try {
            String id = DocumentsContract.getTreeDocumentId(tree);
            String name = name(context, DocumentsContract.buildDocumentUriUsingTree(tree, id));
            copyTree(context, tree, id, root, cancelled, new HashSet<>(), 0);
            check(cancelled);
            if (!new File(root, "data/pack/script.cpz").isFile()
                || (!new File(root, "data/pack/start.ps3").isFile() && !new File(root, "data/pack/start.ps2").isFile())) {
                throw new IOException("请选择包含 data/pack 的游戏根目录");
            }
            return new Imported(path, name);
        } catch (Exception e) { remove(root); throw new IOException(e.getMessage(), e); }
    }

    private static void copyTree(Context context, Uri tree, String id, File destination,
        AtomicBoolean cancelled, Set<String> visited, int depth) throws IOException {
        check(cancelled);
        if (depth > 32 || visited.size() >= 100000 || !visited.add(id)) throw new IOException("目录层级、数量异常或存在循环");
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id);
        String[] columns = {DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE};
        try (Cursor cursor = context.getContentResolver().query(children, columns, null, null, null)) {
            if (cursor == null) throw new IOException("无法读取目录");
            while (cursor.moveToNext()) {
                check(cancelled);
                String childId = cursor.getString(0), childName = cursor.getString(1), mime = cursor.getString(2);
                if (childName == null || childName.isEmpty() || childName.equals(".") || childName.equals("..")
                    || childName.contains("/") || childName.contains("\\")) throw new IOException("文件名无效");
                File child = new File(destination, childName);
                if (child.exists()) throw new IOException("目录包含重复文件名");
                if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) {
                    if (!child.mkdir()) throw new IOException("无法创建子目录");
                    copyTree(context, tree, childId, child, cancelled, visited, depth + 1);
                } else {
                    copy(context, DocumentsContract.buildDocumentUriUsingTree(tree, childId), child, cancelled, Long.MAX_VALUE);
                }
            }
        }
    }

    static Imported font(Context context, Uri uri, AtomicBoolean cancelled) throws IOException {
        File directory = new File(context.getFilesDir(), "fonts");
        if (!directory.isDirectory() && !directory.mkdirs()) throw new IOException("无法创建字体目录");
        String path = "fonts/font-" + UUID.randomUUID();
        File file = new File(context.getFilesDir(), path);
        try { copy(context, uri, file, cancelled, 32 * 1024 * 1024); return new Imported(path, name(context, uri)); }
        catch (Exception e) { remove(file); throw new IOException(e.getMessage(), e); }
    }

    private static String name(Context context, Uri uri) {
        try (Cursor cursor = context.getContentResolver().query(uri, new String[]{OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) return cursor.getString(0);
        } catch (Exception ignored) { }
        return "已导入";
    }

    private static void copy(Context context, Uri source, File target, AtomicBoolean cancelled, long limit) throws IOException {
        try (InputStream input = context.getContentResolver().openInputStream(source);
             FileOutputStream output = new FileOutputStream(target)) {
            if (input == null) throw new IOException("无法读取文件");
            byte[] buffer = new byte[256 * 1024];
            long total = 0;
            int count;
            while ((count = input.read(buffer)) != -1) {
                check(cancelled);
                total += count;
                if (total > limit) throw new IOException("字体文件不能超过 32 MiB");
                if (target.getParentFile().getUsableSpace() < count + 16L * 1024 * 1024) throw new IOException("存储空间不足");
                output.write(buffer, 0, count);
            }
            output.getFD().sync();
        }
    }

    private static void check(AtomicBoolean cancelled) throws IOException {
        if (cancelled.get() || Thread.currentThread().isInterrupted()) throw new IOException("已取消");
    }

    static void remove(File file) {
        File[] children = file.listFiles();
        if (children != null) for (File child : children) remove(child);
        file.delete();
    }
}
