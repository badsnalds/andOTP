package org.shadowice.andotp.passkey;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;

import androidx.annotation.RequiresApi;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.security.KeyPairGenerator;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.interfaces.ECPublicKey;
import java.security.spec.ECGenParameterSpec;
import java.util.ArrayList;
import java.util.List;

/**
 * Device-bound passkeys: each private key is a P-256 key in the Android Keystore that never
 * leaves it and needs a biometric or screen-lock check for every signature. The account details
 * (site, user name, credential id) are kept in {@code passkeys.json} in private app storage.
 */
@RequiresApi(34)
public final class PasskeyStore {
    private static final String FILE = "passkeys.json";
    private static final String KEYSTORE = "AndroidKeyStore";
    private static final String ALIAS_PREFIX = "passkey-";

    /** One stored passkey. */
    public static final class Passkey {
        public final String credentialId; // base64url
        public final String rpId;
        public final String rpName;
        public final String userId; // base64url user handle
        public final String userName;
        public final String displayName;
        public final long created; // ms since the epoch

        Passkey(String credentialId, String rpId, String rpName, String userId, String userName,
                String displayName, long created) {
            this.credentialId = credentialId;
            this.rpId = rpId;
            this.rpName = rpName;
            this.userId = userId;
            this.userName = userName;
            this.displayName = displayName;
            this.created = created;
        }

        String alias() {
            return ALIAS_PREFIX + credentialId;
        }

        JSONObject toJson() throws JSONException {
            return new JSONObject()
                    .put("credentialId", credentialId)
                    .put("rpId", rpId)
                    .put("rpName", rpName)
                    .put("userId", userId)
                    .put("userName", userName)
                    .put("displayName", displayName)
                    .put("created", created);
        }

        static Passkey fromJson(JSONObject o) {
            return new Passkey(o.optString("credentialId"), o.optString("rpId"), o.optString("rpName"),
                    o.optString("userId"), o.optString("userName"), o.optString("displayName"),
                    o.optLong("created"));
        }
    }

    private PasskeyStore() {}

    public static synchronized List<Passkey> list(Context context) {
        List<Passkey> result = new ArrayList<>();
        File file = new File(context.getFilesDir(), FILE);
        if (!file.exists()) {
            return result;
        }
        try (InputStream in = new FileInputStream(file)) {
            JSONArray array = new JSONArray(new String(in.readAllBytes(), StandardCharsets.UTF_8));
            for (int i = 0; i < array.length(); i++) {
                result.add(Passkey.fromJson(array.getJSONObject(i)));
            }
        } catch (Exception e) {
            // Unreadable list: treat as empty rather than failing every sign-in.
        }
        return result;
    }

    /** Passkeys for {@code rpId}, optionally limited to the given credential ids. */
    public static List<Passkey> forRp(Context context, String rpId, List<String> allowed) {
        List<Passkey> result = new ArrayList<>();
        for (Passkey passkey : list(context)) {
            if (passkey.rpId.equals(rpId) && (allowed.isEmpty() || allowed.contains(passkey.credentialId))) {
                result.add(passkey);
            }
        }
        return result;
    }

    public static Passkey find(Context context, String credentialId) {
        for (Passkey passkey : list(context)) {
            if (passkey.credentialId.equals(credentialId)) {
                return passkey;
            }
        }
        return null;
    }

    /** Creates the Keystore key for a new passkey; {@link #add} records it once the response is built. */
    public static ECPublicKey generateKey(byte[] credentialId) throws Exception {
        KeyPairGenerator generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, KEYSTORE);
        generator.initialize(new KeyGenParameterSpec.Builder(ALIAS_PREFIX + WebAuthn.b64url(credentialId),
                KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(new ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setUserAuthenticationRequired(true)
                .setUserAuthenticationParameters(0,
                        KeyProperties.AUTH_BIOMETRIC_STRONG | KeyProperties.AUTH_DEVICE_CREDENTIAL)
                // A new fingerprint must not make every passkey unusable.
                .setInvalidatedByBiometricEnrollment(false)
                .build());
        return (ECPublicKey) generator.generateKeyPair().getPublic();
    }

    public static PrivateKey privateKey(Passkey passkey) throws Exception {
        KeyStore keyStore = KeyStore.getInstance(KEYSTORE);
        keyStore.load(null);
        PrivateKey key = (PrivateKey) keyStore.getKey(passkey.alias(), null);
        if (key == null) {
            throw new IllegalStateException("The key of this passkey is missing");
        }
        return key;
    }

    public static synchronized void add(Context context, byte[] credentialId, String rpId, String rpName,
                                        byte[] userId, String userName, String displayName) throws Exception {
        List<Passkey> passkeys = list(context);
        passkeys.add(new Passkey(WebAuthn.b64url(credentialId), rpId, rpName, WebAuthn.b64url(userId), userName,
                displayName, System.currentTimeMillis()));
        save(context, passkeys);
    }

    /** Removes a passkey and its key. The site still lists it until removed there too. */
    public static synchronized void delete(Context context, String credentialId) {
        List<Passkey> passkeys = list(context);
        passkeys.removeIf(p -> p.credentialId.equals(credentialId));
        try {
            save(context, passkeys);
        } catch (Exception ignored) {
            // Keep going: the key below is what matters.
        }
        deleteKey(ALIAS_PREFIX + credentialId);
    }

    /** Removes a key generated for a passkey that was never completed. */
    public static void deleteKey(byte[] credentialId) {
        deleteKey(ALIAS_PREFIX + WebAuthn.b64url(credentialId));
    }

    private static void deleteKey(String alias) {
        try {
            KeyStore keyStore = KeyStore.getInstance(KEYSTORE);
            keyStore.load(null);
            keyStore.deleteEntry(alias);
        } catch (Exception ignored) {
            // Already gone.
        }
    }

    /** The passkey list as JSON for the app's UI (no key material). */
    public static String listJson(Context context) {
        JSONArray array = new JSONArray();
        try {
            for (Passkey passkey : list(context)) {
                array.put(passkey.toJson());
            }
        } catch (JSONException ignored) {
            // Cannot happen with these value types.
        }
        return array.toString();
    }

    private static void save(Context context, List<Passkey> passkeys) throws Exception {
        JSONArray array = new JSONArray();
        for (Passkey passkey : passkeys) {
            array.put(passkey.toJson());
        }
        File file = new File(context.getFilesDir(), FILE);
        File tmp = new File(context.getFilesDir(), FILE + ".tmp");
        try (OutputStream out = new FileOutputStream(tmp)) {
            out.write(array.toString().getBytes(StandardCharsets.UTF_8));
        }
        if (!tmp.renameTo(file)) {
            throw new IllegalStateException("Cannot save the passkey list");
        }
    }
}
