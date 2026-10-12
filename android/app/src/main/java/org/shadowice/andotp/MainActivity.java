package org.shadowice.andotp;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.database.Cursor;
import android.graphics.Insets;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.PersistableBundle;
import android.provider.OpenableColumns;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowManager;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.Arrays;

/**
 * Hosts the Rust/egui app (libandotp_android.so) and provides the Android services it cannot
 * reach on its own. The methods below are called from Rust through JNI
 * (crates/andotp-gui/src/android.rs); results are polled, never pushed.
 */
public class MainActivity extends NativeActivity {
    private static final int REQUEST_OPEN = 1;
    private static final int REQUEST_CREATE = 2;

    private byte[] opened;
    private String message;
    private byte[] pendingExport;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // Keep tokens out of screenshots and the recent-apps preview (release builds only, so
        // automated tests can still take screenshots of debug builds).
        boolean debuggable = (getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0;
        if (!debuggable) {
            getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        }
    }

    /** Copies a token, marked as sensitive so it is hidden from clipboard previews. */
    public void copyToClipboard(String text) {
        runOnUiThread(() -> {
            ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            ClipData clip = ClipData.newPlainText("andOTP", text);
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                PersistableBundle extras = new PersistableBundle();
                extras.putBoolean("android.content.extra.IS_SENSITIVE", true);
                clip.getDescription().setExtras(extras);
            }
            clipboard.setPrimaryClip(clip);
        });
    }

    /** Shows the system file picker; the file's bytes become available via {@link #takeOpened}. */
    public void openDocument(String mime) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(mime);
            startActivityForResult(intent, REQUEST_OPEN);
        });
    }

    /** Lets the user choose where to save {@code data}; the outcome is reported via {@link #takeMessage}. */
    public void createDocument(String name, String mime, byte[] data) {
        synchronized (this) {
            clearPendingExport();
            pendingExport = data;
        }
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(mime);
            intent.putExtra(Intent.EXTRA_TITLE, name);
            startActivityForResult(intent, REQUEST_CREATE);
        });
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQUEST_OPEN && requestCode != REQUEST_CREATE) {
            return;
        }
        Uri uri = (resultCode == RESULT_OK && data != null) ? data.getData() : null;
        if (uri == null) {
            synchronized (this) {
                clearPendingExport();
            }
            post("-cancelled");
            return;
        }
        // File I/O off the UI thread.
        new Thread(() -> {
            try {
                if (requestCode == REQUEST_OPEN) {
                    byte[] bytes = readAll(uri);
                    synchronized (this) {
                        opened = bytes;
                    }
                } else {
                    byte[] bytes;
                    synchronized (this) {
                        bytes = pendingExport;
                    }
                    if (bytes == null) {
                        return;
                    }
                    try (OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                        out.write(bytes);
                    }
                    post("Saved " + displayName(uri));
                }
            } catch (Exception e) {
                post("!" + e.getMessage());
            } finally {
                synchronized (this) {
                    clearPendingExport();
                }
            }
        }).start();
    }

    /** The bytes of the last picked file, or null. */
    public synchronized byte[] takeOpened() {
        byte[] result = opened;
        opened = null;
        return result;
    }

    /** A status message ("!" prefix: error, "-" prefix: cancelled), or null. */
    public synchronized String takeMessage() {
        String result = message;
        message = null;
        return result;
    }

    /** Heights of the system bars, display cutout and on-screen keyboard in pixels: {top, bottom}. */
    public int[] systemInsets() {
        View decor = getWindow().getDecorView();
        WindowInsets insets = decor.getRootWindowInsets();
        if (insets == null) {
            return new int[] {0, 0};
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Insets bars = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            Insets ime = insets.getInsets(WindowInsets.Type.ime());
            return new int[] {bars.top, Math.max(bars.bottom, ime.bottom)};
        }
        return new int[] {insets.getSystemWindowInsetTop(), insets.getSystemWindowInsetBottom()};
    }

    private synchronized void post(String text) {
        message = text;
    }

    private void clearPendingExport() {
        if (pendingExport != null) {
            Arrays.fill(pendingExport, (byte) 0);
            pendingExport = null;
        }
    }

    private byte[] readAll(Uri uri) throws Exception {
        try (InputStream in = getContentResolver().openInputStream(uri);
             ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[8192];
            int n;
            while ((n = in.read(buffer)) > 0) {
                out.write(buffer, 0, n);
            }
            return out.toByteArray();
        }
    }

    private String displayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                return cursor.getString(0);
            }
        } catch (Exception ignored) {
            // Fall through to the URI.
        }
        return uri.getLastPathSegment();
    }
}
