package org.cmvs.player;

import android.app.AlertDialog;
import android.app.NativeActivity;
import android.content.Intent;
import android.os.Bundle;
import android.text.InputType;
import android.widget.EditText;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.concurrent.atomic.AtomicBoolean;

public final class GameActivity extends NativeActivity {
    private final AtomicBoolean cancelled = new AtomicBoolean();
    private boolean choosingFont;

    /** Called on the Java UI thread by the native platform backend. */
    public void chooseFont() {
        if (choosingFont || isFinishing() || isDestroyed()) return;
        choosingFont = true;
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*");
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try { startActivityForResult(intent, 1); }
        catch (Exception e) { answer("cancel\n"); }
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != 1) return;
        if (result != RESULT_OK || data == null || data.getData() == null) { answer("cancel\n"); return; }
        android.net.Uri uri = data.getData();
        EditText index = new EditText(this);
        index.setInputType(InputType.TYPE_CLASS_NUMBER);
        index.setText("0");
        new AlertDialog.Builder(this).setTitle("字体集合索引").setMessage("普通字体使用 0")
            .setView(index).setNegativeButton("取消", (dialog, which) -> answer("cancel\n"))
            .setOnCancelListener(dialog -> answer("cancel\n"))
            .setPositiveButton("确定", (dialog, which) -> {
                final int face;
                try { face = Integer.parseInt(index.getText().toString()); if (face < 0 || face > 1024) throw new IllegalArgumentException(); }
                catch (Exception e) { answer("cancel\n"); return; }
                new Thread(() -> {
                    try {
                        GameImport.Imported font = GameImport.font(this, uri, cancelled);
                        answer(font.path + "\n" + face + "\n");
                    } catch (Exception e) { answer("cancel\n"); }
                }, "font-import").start();
            }).show();
    }

    private void answer(String value) {
        if (isDestroyed()) return;
        try {
            File temporary = new File(getFilesDir(), "font-result.tmp");
            Files.write(temporary.toPath(), value.getBytes(StandardCharsets.UTF_8));
            Files.move(temporary.toPath(), new File(getFilesDir(), "font-result.cfg").toPath(), StandardCopyOption.REPLACE_EXISTING);
        } catch (Exception ignored) { }
        runOnUiThread(() -> choosingFont = false);
    }

    @Override protected void onDestroy() {
        cancelled.set(true);
        super.onDestroy();
    }
}
