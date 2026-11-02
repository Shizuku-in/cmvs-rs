package org.cmvs.player;

import android.app.Activity;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Bundle;
import android.text.InputType;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

public final class LauncherActivity extends Activity {
    private static final int GAME = 1, FONT = 2;
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private final AtomicBoolean cancelled = new AtomicBoolean();
    private SharedPreferences settings;
    private TextView gameLabel, fontLabel, status;
    private EditText face;
    private Button gameButton, fontButton, startButton, cancelButton;
    private boolean busy;

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        // Reopening the app should return to the existing game, rather than
        // put a second importer on top of resources that are still in use.
        if (!isTaskRoot()) { finish(); return; }
        settings = getSharedPreferences("launcher", MODE_PRIVATE);
        ScrollView scroll = new ScrollView(this);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        int pad = Math.round(24 * getResources().getDisplayMetrics().density);
        layout.setPadding(pad, pad, pad, pad);
        scroll.addView(layout);
        text(layout, "CMVS").setTextSize(24);
        text(layout, "当前支持 Shift-JIS 游戏，支持 BGM、语音和音效。\n导入会复制游戏文件并占用额外空间，原目录不会修改。");
        gameLabel = text(layout, "");
        gameButton = button(layout, "导入游戏目录", () -> pick(GAME));
        fontLabel = text(layout, "");
        fontButton = button(layout, "选择字体", () -> pick(FONT));
        text(layout, "字体集合索引（普通字体填 0）");
        face = new EditText(this);
        face.setInputType(InputType.TYPE_CLASS_NUMBER);
        face.setText(Integer.toString(settings.getInt("face", 0)));
        layout.addView(face);
        startButton = button(layout, "启动", this::launch);
        cancelButton = button(layout, "取消导入", () -> cancelled.set(true));
        status = text(layout, "");
        status.setAccessibilityLiveRegion(TextView.ACCESSIBILITY_LIVE_REGION_POLITE);
        setContentView(scroll);
        refresh();
    }

    private TextView text(LinearLayout layout, String value) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setPadding(0, 12, 0, 12);
        layout.addView(view);
        return view;
    }

    private Button button(LinearLayout layout, String title, Runnable action) {
        Button button = new Button(this);
        button.setText(title);
        button.setOnClickListener(v -> action.run());
        layout.addView(button);
        return button;
    }

    private void refresh() {
        gameLabel.setText("游戏：" + settings.getString("gameName", "未导入"));
        fontLabel.setText("字体：" + settings.getString("fontName", "未选择"));
        gameButton.setEnabled(!busy);
        fontButton.setEnabled(!busy);
        face.setEnabled(!busy);
        startButton.setEnabled(!busy && selected("game").isDirectory() && selected("font").isFile());
        cancelButton.setVisibility(busy ? Button.VISIBLE : Button.GONE);
    }

    private File selected(String key) {
        return new File(getFilesDir(), settings.getString(key, "not-selected"));
    }

    private void pick(int kind) {
        Intent intent = new Intent(kind == GAME ? Intent.ACTION_OPEN_DOCUMENT_TREE : Intent.ACTION_OPEN_DOCUMENT);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        if (kind == FONT) { intent.setType("*/*"); intent.addCategory(Intent.CATEGORY_OPENABLE); }
        try { startActivityForResult(intent, kind); }
        catch (android.content.ActivityNotFoundException e) { status.setText("没有可用的系统文件选择器。"); }
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (result != RESULT_OK || data == null || data.getData() == null || busy) return;
        Uri uri = data.getData();
        busy = true;
        cancelled.set(false);
        status.setText(request == GAME ? "正在导入游戏…" : "正在导入字体…");
        refresh();
        worker.execute(() -> {
            try {
                GameImport.Imported imported = request == GAME
                    ? GameImport.game(this, uri, cancelled)
                    : GameImport.font(this, uri, cancelled);
                String key = request == GAME ? "game" : "font";
                File previous = selected(key);
                // Publish only a completed import; failed imports keep the previous selection.
                boolean committed = settings.edit().putString(key, imported.path)
                    .putString(key + "Name", imported.name).commit();
                if (!committed) { GameImport.remove(new File(getFilesDir(), imported.path)); throw new java.io.IOException("无法保存导入配置"); }
                String prefix = request == GAME ? "games/" : "fonts/";
                String old = getFilesDir().toPath().relativize(previous.toPath()).toString().replace(File.separatorChar, '/');
                if (old.startsWith(prefix) && !old.equals(imported.path)) GameImport.remove(previous);
                runOnUiThread(() -> status.setText("导入完成。"));
            } catch (Exception e) {
                runOnUiThread(() -> status.setText(cancelled.get() ? "已取消，原有选择保留。" : "导入失败：" + e.getMessage()));
            } finally {
                runOnUiThread(() -> { busy = false; refresh(); });
            }
        });
    }

    private void launch() {
        try {
            int index = Integer.parseInt(face.getText().toString());
            if (index < 0 || index > 1024) throw new IllegalArgumentException();
            settings.edit().putInt("face", index).apply();
            String config = settings.getString("game", "") + "\n" + settings.getString("font", "") + "\n" + index + "\n";
            File temporary = new File(getFilesDir(), "launch.tmp");
            Files.write(temporary.toPath(), config.getBytes(StandardCharsets.UTF_8));
            Files.move(temporary.toPath(), new File(getFilesDir(), "launch.cfg").toPath(), java.nio.file.StandardCopyOption.REPLACE_EXISTING);
            Files.deleteIfExists(new File(getFilesDir(), "launch-error.txt").toPath());
            startActivity(new Intent(this, GameActivity.class));
        } catch (NumberFormatException e) { status.setText("字体集合索引需要填写整数。"); }
        catch (IllegalArgumentException e) { status.setText("字体集合索引需要在 0 到 1024 之间。"); }
        catch (Exception e) { status.setText("无法启动：" + e.getMessage()); }
    }

    @Override protected void onResume() {
        super.onResume();
        if (status == null) return;
        File error = new File(getFilesDir(), "launch-error.txt");
        if (error.isFile()) {
            try { status.setText(new String(Files.readAllBytes(error.toPath()), StandardCharsets.UTF_8)); Files.deleteIfExists(error.toPath()); }
            catch (Exception ignored) { status.setText("游戏运行停止，请检查游戏版本和字体。"); }
        }
    }

    @Override public void onBackPressed() {
        if (busy) { cancelled.set(true); return; }
        super.onBackPressed();
    }

    @Override protected void onDestroy() {
        cancelled.set(true);
        worker.shutdownNow();
        super.onDestroy();
    }
}
