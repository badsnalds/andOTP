package org.shadowice.andotp;

import android.Manifest;
import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.ImageFormat;
import android.hardware.biometrics.BiometricManager;
import android.hardware.biometrics.BiometricPrompt;
import android.graphics.Insets;
import android.hardware.camera2.CameraAccessException;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.PersistableBundle;
import android.provider.OpenableColumns;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyPermanentlyInvalidatedException;
import android.security.keystore.KeyProperties;
import android.util.Size;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowManager;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.util.Arrays;
import java.security.KeyStore;
import java.util.Collections;

import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * Hosts the Rust/egui app (libandotp_android.so) and provides the Android services it cannot
 * reach on its own. The methods below are called from Rust through JNI
 * (crates/andotp-gui/src/android.rs); results are polled, never pushed.
 */
public class MainActivity extends NativeActivity {
    private static final int REQUEST_OPEN = 1;
    private static final int REQUEST_CREATE = 2;
    private static final int REQUEST_CAMERA = 3;
    /** Frames larger than this (either side) are not needed to read a QR code. */
    private static final int MAX_FRAME_WIDTH = 1280;
    private static final int MAX_FRAME_HEIGHT = 960;

    private byte[] opened;
    private String message;
    private byte[] pendingExport;

    // Camera state. cameraWanted is set while Rust shows the scanner; the device is closed while
    // the activity is paused and reopened on resume.
    private boolean cameraWanted;
    private boolean cameraOpening;
    private boolean resumed;
    private CameraDevice camera;
    private CameraCaptureSession cameraSession;
    private ImageReader imageReader;
    private HandlerThread cameraThread;
    private Handler cameraHandler;
    private int sensorRotation;
    private byte[] cameraFrame;
    private String cameraError;

    // Fingerprint unlock: the vault password, encrypted with a Keystore key that needs a strong
    // biometric for every use, lives in BIOMETRIC_FILE. Results are polled from Rust.
    private static final String BIOMETRIC_KEY = "andotp-vault-password";
    private static final String BIOMETRIC_FILE = "biometric.bin";
    private byte[] biometricPassword;
    private String biometricError;

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

    /** Starts streaming frames from the back camera, asking for permission first if needed. */
    public void startCamera() {
        runOnUiThread(() -> {
            synchronized (this) {
                cameraWanted = true;
                cameraFrame = null;
                cameraError = null;
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M
                    && checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(new String[] {Manifest.permission.CAMERA}, REQUEST_CAMERA);
                return;
            }
            openCamera();
        });
    }

    public void stopCamera() {
        runOnUiThread(() -> {
            synchronized (this) {
                cameraWanted = false;
                cameraFrame = null;
            }
            closeCamera();
        });
    }

    /** The latest frame: width, height and sensor rotation (big-endian ints), then the luma plane; or null. */
    public synchronized byte[] takeCameraFrame() {
        byte[] result = cameraFrame;
        cameraFrame = null;
        return result;
    }

    public synchronized String takeCameraError() {
        String result = cameraError;
        cameraError = null;
        return result;
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode != REQUEST_CAMERA) {
            return;
        }
        if (grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            openCamera();
        } else {
            cameraFailed("Camera permission was denied");
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        resumed = true;
        openCamera();
    }

    @Override
    protected void onPause() {
        resumed = false;
        closeCamera();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        closeCamera();
        // NativeActivity.onDestroy waits for the Rust thread (android_main) to return, but the
        // winit event loop never returns on Android, so the main thread would hang until Android
        // reports "not responding" and kills the app. That happens on Back and whenever Android
        // recreates the activity (e.g. after resources change, which apps cannot opt out of).
        // End the process instead: nothing is lost, as the vault is written on every change and
        // the settings were saved when the window went away. If the activity was visible, Android
        // starts it again in a fresh process.
        android.os.Process.killProcess(android.os.Process.myPid());
        super.onDestroy();
    }

    private synchronized void cameraFailed(String text) {
        cameraError = text;
        cameraWanted = false;
    }

    /** Opens the camera if the scanner wants it, it is not open yet and we may use it. UI thread. */
    private void openCamera() {
        synchronized (this) {
            if (!cameraWanted) {
                return;
            }
        }
        if (!resumed || camera != null || cameraOpening) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M
                && checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            return;
        }
        try {
            CameraManager manager = (CameraManager) getSystemService(CAMERA_SERVICE);
            String id = null;
            CameraCharacteristics chosen = null;
            for (String candidate : manager.getCameraIdList()) {
                CameraCharacteristics c = manager.getCameraCharacteristics(candidate);
                Integer facing = c.get(CameraCharacteristics.LENS_FACING);
                if (id == null || (facing != null && facing == CameraCharacteristics.LENS_FACING_BACK)) {
                    id = candidate;
                    chosen = c;
                    if (facing != null && facing == CameraCharacteristics.LENS_FACING_BACK) {
                        break;
                    }
                }
            }
            if (id == null) {
                cameraFailed("This device has no camera");
                return;
            }
            Integer orientation = chosen.get(CameraCharacteristics.SENSOR_ORIENTATION);
            sensorRotation = orientation == null ? 0 : orientation;
            Size size = pickSize(chosen.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP));

            if (cameraThread == null) {
                cameraThread = new HandlerThread("camera");
                cameraThread.start();
                cameraHandler = new Handler(cameraThread.getLooper());
            }
            imageReader = ImageReader.newInstance(size.getWidth(), size.getHeight(), ImageFormat.YUV_420_888, 2);
            imageReader.setOnImageAvailableListener(this::onFrame, cameraHandler);

            cameraOpening = true;
            manager.openCamera(id, new CameraDevice.StateCallback() {
                @Override
                public void onOpened(CameraDevice device) {
                    cameraOpening = false;
                    camera = device;
                    boolean wanted;
                    synchronized (MainActivity.this) {
                        wanted = cameraWanted;
                    }
                    if (!wanted || imageReader == null) {
                        // Stopped or paused while opening; reopen if it is wanted again by now.
                        closeCamera();
                        openCamera();
                        return;
                    }
                    startPreview();
                }

                @Override
                public void onDisconnected(CameraDevice device) {
                    cameraOpening = false;
                    device.close();
                    if (camera == device) {
                        camera = null;
                    }
                }

                @Override
                public void onError(CameraDevice device, int error) {
                    cameraOpening = false;
                    device.close();
                    if (camera == device) {
                        camera = null;
                    }
                    cameraFailed("The camera is unavailable (error " + error + ")");
                }
            }, new Handler(getMainLooper()));
        } catch (CameraAccessException | SecurityException | IllegalArgumentException e) {
            cameraOpening = false;
            cameraFailed("Cannot open the camera: " + e.getMessage());
        }
    }

    private static Size pickSize(StreamConfigurationMap map) {
        Size best = null;
        Size[] sizes = map == null ? null : map.getOutputSizes(ImageFormat.YUV_420_888);
        if (sizes != null) {
            for (Size s : sizes) {
                if (s.getWidth() > MAX_FRAME_WIDTH || s.getHeight() > MAX_FRAME_HEIGHT) {
                    continue;
                }
                if (best == null || (long) s.getWidth() * s.getHeight() > (long) best.getWidth() * best.getHeight()) {
                    best = s;
                }
            }
        }
        return best != null ? best : new Size(640, 480);
    }

    private void startPreview() {
        try {
            CaptureRequest.Builder request = camera.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW);
            request.addTarget(imageReader.getSurface());
            request.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE);
            CameraDevice device = camera;
            device.createCaptureSession(Collections.singletonList(imageReader.getSurface()),
                    new CameraCaptureSession.StateCallback() {
                        @Override
                        public void onConfigured(CameraCaptureSession session) {
                            if (camera != device) {
                                session.close();
                                return;
                            }
                            cameraSession = session;
                            try {
                                session.setRepeatingRequest(request.build(), null, cameraHandler);
                            } catch (CameraAccessException | IllegalStateException e) {
                                cameraFailed("Cannot start the camera: " + e.getMessage());
                            }
                        }

                        @Override
                        public void onConfigureFailed(CameraCaptureSession session) {
                            cameraFailed("Cannot configure the camera");
                        }
                    }, new Handler(getMainLooper()));
        } catch (CameraAccessException | IllegalStateException e) {
            cameraFailed("Cannot start the camera: " + e.getMessage());
        }
    }

    /** Camera thread: keeps the luma plane of the newest frame. */
    private void onFrame(ImageReader reader) {
        Image image = reader.acquireLatestImage();
        if (image == null) {
            return;
        }
        synchronized (this) {
            // The app has not taken the previous frame yet: skip this one instead of copying
            // frames nobody will look at.
            if (!cameraWanted || cameraFrame != null) {
                image.close();
                return;
            }
        }
        try {
            int width = image.getWidth();
            int height = image.getHeight();
            Image.Plane plane = image.getPlanes()[0];
            ByteBuffer buffer = plane.getBuffer();
            int rowStride = plane.getRowStride();
            byte[] frame = new byte[12 + width * height];
            ByteBuffer header = ByteBuffer.wrap(frame);
            header.putInt(width).putInt(height).putInt(sensorRotation);
            for (int y = 0; y < height; y++) {
                buffer.position(y * rowStride);
                buffer.get(frame, 12 + y * width, width);
            }
            synchronized (this) {
                if (cameraWanted) {
                    cameraFrame = frame;
                }
            }
        } catch (RuntimeException e) {
            // A frame we cannot read; the next one will do.
        } finally {
            image.close();
        }
    }

    /** UI thread. */
    private void closeCamera() {
        if (cameraSession != null) {
            cameraSession.close();
            cameraSession = null;
        }
        if (camera != null) {
            camera.close();
            camera = null;
        }
        if (imageReader != null) {
            imageReader.close();
            imageReader = null;
        }
    }

    /** Whether this device can do fingerprint unlock (Android 11+ with a strong biometric enrolled). */
    public boolean biometricAvailable() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            return false;
        }
        BiometricManager manager = getSystemService(BiometricManager.class);
        return manager != null
                && manager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS;
    }

    /** Whether a password is stored for fingerprint unlock. */
    public boolean biometricEnabled() {
        return new File(getFilesDir(), BIOMETRIC_FILE).exists();
    }

    /** Forgets the stored password and its key. */
    public void biometricDisable() {
        new File(getFilesDir(), BIOMETRIC_FILE).delete();
        try {
            KeyStore keyStore = KeyStore.getInstance("AndroidKeyStore");
            keyStore.load(null);
            keyStore.deleteEntry(BIOMETRIC_KEY);
        } catch (Exception ignored) {
            // Nothing stored.
        }
    }

    /**
     * Stores {@code password} for fingerprint unlock after a fingerprint check. The outcome is
     * reported via {@link #takeMessage}. The array is wiped.
     */
    public void biometricEnroll(byte[] password) {
        runOnUiThread(() -> {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
                Arrays.fill(password, (byte) 0);
                post("!Fingerprint unlock needs Android 11 or later");
                return;
            }
            Cipher cipher;
            try {
                biometricDisable();
                KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
                generator.init(new KeyGenParameterSpec.Builder(BIOMETRIC_KEY,
                        KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                        .setKeySize(256)
                        .setUserAuthenticationRequired(true)
                        .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                        .setInvalidatedByBiometricEnrollment(true)
                        .build());
                SecretKey key = generator.generateKey();
                cipher = Cipher.getInstance("AES/GCM/NoPadding");
                cipher.init(Cipher.ENCRYPT_MODE, key);
            } catch (Exception e) {
                Arrays.fill(password, (byte) 0);
                post("!Cannot set up fingerprint unlock: " + e.getMessage());
                return;
            }
            authenticate(cipher, "Turn on fingerprint unlock", new BiometricPrompt.AuthenticationCallback() {
                @Override
                public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                    try {
                        Cipher c = result.getCryptoObject().getCipher();
                        byte[] encrypted = c.doFinal(password);
                        byte[] iv = c.getIV();
                        try (FileOutputStream out = new FileOutputStream(new File(getFilesDir(), BIOMETRIC_FILE))) {
                            out.write(iv.length);
                            out.write(iv);
                            out.write(encrypted);
                        }
                        post("Fingerprint unlock is on");
                    } catch (Exception e) {
                        biometricDisable();
                        post("!Cannot set up fingerprint unlock: " + e.getMessage());
                    } finally {
                        Arrays.fill(password, (byte) 0);
                    }
                }

                @Override
                public void onAuthenticationError(int code, CharSequence message) {
                    Arrays.fill(password, (byte) 0);
                    post(isCancel(code) ? "-cancelled" : "!" + message);
                }
            }, () -> {
                Arrays.fill(password, (byte) 0);
                post("-cancelled");
            });
        });
    }

    /** Asks for a fingerprint and decrypts the stored password; poll {@link #takeBiometricPassword}. */
    public void biometricUnlock() {
        runOnUiThread(() -> {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
                biometricFailed("Fingerprint unlock needs Android 11 or later");
                return;
            }
            Cipher cipher;
            byte[] encrypted;
            try {
                byte[] file;
                try (InputStream in = new FileInputStream(new File(getFilesDir(), BIOMETRIC_FILE))) {
                    file = readStream(in);
                }
                int ivLength = file[0];
                byte[] iv = Arrays.copyOfRange(file, 1, 1 + ivLength);
                encrypted = Arrays.copyOfRange(file, 1 + ivLength, file.length);
                KeyStore keyStore = KeyStore.getInstance("AndroidKeyStore");
                keyStore.load(null);
                SecretKey key = (SecretKey) keyStore.getKey(BIOMETRIC_KEY, null);
                cipher = Cipher.getInstance("AES/GCM/NoPadding");
                cipher.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(128, iv));
            } catch (KeyPermanentlyInvalidatedException e) {
                biometricDisable();
                biometricFailed("Your fingerprints changed. Unlock with your password and turn fingerprint unlock on again.");
                return;
            } catch (Exception e) {
                biometricFailed("Fingerprint unlock is not available: " + e.getMessage());
                return;
            }
            authenticate(cipher, "Unlock andOTP", new BiometricPrompt.AuthenticationCallback() {
                @Override
                public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                    try {
                        byte[] password = result.getCryptoObject().getCipher().doFinal(encrypted);
                        synchronized (MainActivity.this) {
                            biometricPassword = password;
                        }
                    } catch (Exception e) {
                        biometricFailed("Cannot read the stored password: " + e.getMessage());
                    }
                }

                @Override
                public void onAuthenticationError(int code, CharSequence message) {
                    biometricFailed(isCancel(code) ? "-" : message.toString());
                }
            }, () -> biometricFailed("-"));
        });
    }

    /** The password from a successful {@link #biometricUnlock}, or null. The caller wipes it. */
    public synchronized byte[] takeBiometricPassword() {
        byte[] result = biometricPassword;
        biometricPassword = null;
        return result;
    }

    /** Why {@link #biometricUnlock} failed ("-" = cancelled by the user), or null. */
    public synchronized String takeBiometricError() {
        String result = biometricError;
        biometricError = null;
        return result;
    }

    private synchronized void biometricFailed(String text) {
        biometricError = text;
    }

    private static boolean isCancel(int code) {
        return code == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED
                || code == BiometricPrompt.BIOMETRIC_ERROR_CANCELED
                || code == BiometricPrompt.BIOMETRIC_ERROR_NEGATIVE_BUTTON;
    }

    private void authenticate(Cipher cipher, String title, BiometricPrompt.AuthenticationCallback callback,
                              Runnable onUsePassword) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            return;
        }
        BiometricPrompt prompt = new BiometricPrompt.Builder(this)
                .setTitle(title)
                .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                .setNegativeButton("Use password", getMainExecutor(), (dialog, which) -> onUsePassword.run())
                .build();
        prompt.authenticate(new BiometricPrompt.CryptoObject(cipher), new CancellationSignal(), getMainExecutor(), callback);
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
        try (InputStream in = getContentResolver().openInputStream(uri)) {
            return readStream(in);
        }
    }

    private static byte[] readStream(InputStream in) throws Exception {
        try (ByteArrayOutputStream out = new ByteArrayOutputStream()) {
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
