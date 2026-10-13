package org.shadowice.andotp.passkey;

import android.app.Activity;
import android.content.Intent;
import android.hardware.biometrics.BiometricManager;
import android.hardware.biometrics.BiometricPrompt;
import android.net.Uri;
import android.os.Bundle;
import android.os.CancellationSignal;

import androidx.annotation.RequiresApi;
import androidx.credentials.CreatePublicKeyCredentialRequest;
import androidx.credentials.CreatePublicKeyCredentialResponse;
import androidx.credentials.CredentialOption;
import androidx.credentials.GetCredentialResponse;
import androidx.credentials.GetPublicKeyCredentialOption;
import androidx.credentials.PublicKeyCredential;
import androidx.credentials.exceptions.CreateCredentialCancellationException;
import androidx.credentials.exceptions.CreateCredentialException;
import androidx.credentials.exceptions.CreateCredentialUnknownException;
import androidx.credentials.exceptions.GetCredentialCancellationException;
import androidx.credentials.exceptions.GetCredentialException;
import androidx.credentials.exceptions.GetCredentialUnknownException;
import androidx.credentials.exceptions.domerrors.InvalidStateError;
import androidx.credentials.exceptions.domerrors.NotSupportedError;
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException;
import androidx.credentials.provider.CallingAppInfo;
import androidx.credentials.provider.PendingIntentHandler;
import androidx.credentials.provider.ProviderCreateCredentialRequest;
import androidx.credentials.provider.ProviderGetCredentialRequest;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.security.Signature;
import java.security.interfaces.ECPublicKey;

/**
 * Creates a passkey or signs in with one, after a fingerprint or screen-lock check. Opened by
 * the Credential Manager through the entries {@link PasskeyService} offers; shows nothing but
 * the system's authentication prompt.
 */
@RequiresApi(34)
public class PasskeyActivity extends Activity {
    /** Browsers allowed to pass a web origin (Google's list of privileged browsers, an asset). */
    private static final String PRIVILEGED_BROWSERS = "privileged_browsers.json";
    private static final int AUTHENTICATORS =
            BiometricManager.Authenticators.BIOMETRIC_STRONG | BiometricManager.Authenticators.DEVICE_CREDENTIAL;

    private final CancellationSignal cancel = new CancellationSignal();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        String action = getIntent().getAction();
        if (PasskeyService.ACTION_CREATE.equals(action)) {
            create();
        } else if (PasskeyService.ACTION_GET.equals(action)) {
            get();
        } else {
            finish();
        }
    }

    @Override
    protected void onDestroy() {
        cancel.cancel();
        super.onDestroy();
    }

    // --- creating a passkey -----------------------------------------------------------------

    private void create() {
        try {
            ProviderCreateCredentialRequest request = PendingIntentHandler.retrieveProviderCreateCredentialRequest(getIntent());
            if (request == null || !(request.getCallingRequest() instanceof CreatePublicKeyCredentialRequest)) {
                throw new IllegalArgumentException("Not a passkey request");
            }
            CreatePublicKeyCredentialRequest publicKey = (CreatePublicKeyCredentialRequest) request.getCallingRequest();
            JSONObject json = new JSONObject(publicKey.getRequestJson());
            Caller caller = caller(request.getCallingAppInfo(), publicKey.getClientDataHash());

            JSONObject rp = json.getJSONObject("rp");
            String rpId = rp.optString("id", caller.host());
            caller.checkRpId(rpId);
            String rpName = rp.optString("name", rpId);
            JSONObject user = json.getJSONObject("user");
            byte[] userId = WebAuthn.fromB64url(user.getString("id"));
            String userName = user.optString("name");
            String displayName = user.optString("displayName");
            String challenge = json.getString("challenge");

            if (!supportsEs256(json.optJSONArray("pubKeyCredParams"))) {
                failCreate(new CreatePublicKeyCredentialDomException(new NotSupportedError(),
                        "andOTP passkeys use ES256, which this site does not accept"));
                return;
            }
            for (String excluded : PasskeyService.allowedIds(new JSONObject().put("allowCredentials",
                    json.optJSONArray("excludeCredentials")))) {
                PasskeyStore.Passkey existing = PasskeyStore.find(this, excluded);
                if (existing != null && existing.rpId.equals(rpId)) {
                    failCreate(new CreatePublicKeyCredentialDomException(new InvalidStateError(),
                            "andOTP already has a passkey for this account"));
                    return;
                }
            }

            String account = userName.isEmpty() ? displayName : userName;
            prompt("Create a passkey", rpName + (account.isEmpty() ? "" : " · " + account), null,
                    new Prompt() {
                        @Override
                        public void succeeded(Signature unused) {
                            byte[] credentialId = new byte[16];
                            new SecureRandom().nextBytes(credentialId);
                            try {
                                ECPublicKey key = PasskeyStore.generateKey(credentialId);
                                byte[] authData = WebAuthn.registrationAuthData(rpId, credentialId, key);
                                byte[] clientData = caller.clientDataJson("webauthn.create", challenge);
                                String response = WebAuthn.registrationResponse(credentialId, clientData, authData, key);
                                PasskeyStore.add(PasskeyActivity.this, credentialId, rpId, rpName, userId, userName,
                                        displayName);
                                Intent result = new Intent();
                                PendingIntentHandler.setCreateCredentialResponse(result,
                                        new CreatePublicKeyCredentialResponse(response));
                                finishWith(result);
                            } catch (Exception e) {
                                PasskeyStore.deleteKey(credentialId);
                                failCreate(new CreateCredentialUnknownException(
                                        "Cannot create the passkey: " + e.getMessage()));
                            }
                        }

                        @Override
                        public void failed(boolean cancelled, CharSequence message) {
                            failCreate(cancelled ? new CreateCredentialCancellationException(message)
                                    : new CreateCredentialUnknownException(message));
                        }
                    });
        } catch (Exception e) {
            failCreate(new CreateCredentialUnknownException(e.getMessage()));
        }
    }

    private static boolean supportsEs256(JSONArray params) {
        if (params == null || params.length() == 0) {
            return true; // WebAuthn defaults to ES256 and RS256
        }
        for (int i = 0; i < params.length(); i++) {
            JSONObject param = params.optJSONObject(i);
            if (param != null && param.optInt("alg") == WebAuthn.ES256) {
                return true;
            }
        }
        return false;
    }

    private void failCreate(CreateCredentialException exception) {
        Intent result = new Intent();
        PendingIntentHandler.setCreateCredentialException(result, exception);
        finishWith(result);
    }

    // --- signing in -------------------------------------------------------------------------

    private void get() {
        try {
            ProviderGetCredentialRequest request = PendingIntentHandler.retrieveProviderGetCredentialRequest(getIntent());
            PasskeyStore.Passkey passkey = PasskeyStore.find(this,
                    getIntent().getStringExtra(PasskeyService.EXTRA_CREDENTIAL_ID));
            if (request == null || passkey == null) {
                throw new IllegalArgumentException("This passkey no longer exists");
            }
            GetPublicKeyCredentialOption option = null;
            for (CredentialOption candidate : request.getCredentialOptions()) {
                if (candidate instanceof GetPublicKeyCredentialOption) {
                    option = (GetPublicKeyCredentialOption) candidate;
                    break;
                }
            }
            if (option == null) {
                throw new IllegalArgumentException("Not a passkey request");
            }
            JSONObject json = new JSONObject(option.getRequestJson());
            Caller caller = caller(request.getCallingAppInfo(), option.getClientDataHash());
            String rpId = json.optString("rpId", caller.host());
            caller.checkRpId(rpId);
            if (!passkey.rpId.equals(rpId)) {
                throw new IllegalArgumentException("This passkey belongs to " + passkey.rpId);
            }
            String challenge = json.getString("challenge");

            Signature signature = Signature.getInstance("SHA256withECDSA");
            signature.initSign(PasskeyStore.privateKey(passkey));
            String account = passkey.userName.isEmpty() ? passkey.displayName : passkey.userName;
            prompt("Sign in with a passkey", passkey.rpName + (account.isEmpty() ? "" : " · " + account),
                    signature, new Prompt() {
                        @Override
                        public void succeeded(Signature unlocked) {
                            try {
                                byte[] authData = WebAuthn.assertionAuthData(rpId);
                                byte[] clientData = caller.clientDataJson("webauthn.get", challenge);
                                unlocked.update(WebAuthn.signedData(authData, caller.clientDataHash(clientData)));
                                String response = WebAuthn.authenticationResponse(
                                        WebAuthn.fromB64url(passkey.credentialId), clientData, authData,
                                        unlocked.sign(), WebAuthn.fromB64url(passkey.userId));
                                Intent result = new Intent();
                                PendingIntentHandler.setGetCredentialResponse(result,
                                        new GetCredentialResponse(new PublicKeyCredential(response)));
                                finishWith(result);
                            } catch (Exception e) {
                                failGet(new GetCredentialUnknownException("Cannot sign in: " + e.getMessage()));
                            }
                        }

                        @Override
                        public void failed(boolean cancelled, CharSequence message) {
                            failGet(cancelled ? new GetCredentialCancellationException(message)
                                    : new GetCredentialUnknownException(message));
                        }
                    });
        } catch (Exception e) {
            failGet(new GetCredentialUnknownException(e.getMessage()));
        }
    }

    private void failGet(GetCredentialException exception) {
        Intent result = new Intent();
        PendingIntentHandler.setGetCredentialException(result, exception);
        finishWith(result);
    }

    // --- shared -----------------------------------------------------------------------------

    private void finishWith(Intent result) {
        setResult(RESULT_OK, result);
        finish();
    }

    private interface Prompt {
        void succeeded(Signature signature);

        void failed(boolean cancelled, CharSequence message);
    }

    /** Shows the fingerprint / screen-lock prompt; with a signature, it unlocks that key for one use. */
    private void prompt(String title, String subtitle, Signature signature, Prompt callback) {
        BiometricPrompt prompt = new BiometricPrompt.Builder(this)
                .setTitle(title)
                .setSubtitle(subtitle)
                .setAllowedAuthenticators(AUTHENTICATORS)
                .build();
        BiometricPrompt.AuthenticationCallback authCallback = new BiometricPrompt.AuthenticationCallback() {
            @Override
            public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                BiometricPrompt.CryptoObject crypto = result.getCryptoObject();
                callback.succeeded(crypto != null ? crypto.getSignature() : null);
            }

            @Override
            public void onAuthenticationError(int code, CharSequence message) {
                boolean cancelled = code == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED
                        || code == BiometricPrompt.BIOMETRIC_ERROR_CANCELED;
                callback.failed(cancelled, message);
            }
        };
        if (signature != null) {
            prompt.authenticate(new BiometricPrompt.CryptoObject(signature), cancel, getMainExecutor(), authCallback);
        } else {
            prompt.authenticate(cancel, getMainExecutor(), authCallback);
        }
    }

    /** Who is asking: a privileged browser (with a web origin) or an app. */
    private static final class Caller {
        final String origin;
        /** Set when a browser computed the client data itself (we then sign its hash). */
        final byte[] browserClientDataHash;
        final boolean browser;

        Caller(String origin, byte[] browserClientDataHash, boolean browser) {
            this.origin = origin;
            this.browserClientDataHash = browserClientDataHash;
            this.browser = browser;
        }

        String host() {
            return browser ? Uri.parse(origin).getHost() : "";
        }

        /** A browser's page may only use passkeys of its own domain (or a parent domain). */
        void checkRpId(String rpId) {
            if (rpId == null || rpId.isEmpty()) {
                throw new IllegalArgumentException("The request names no site");
            }
            if (browser) {
                String host = host();
                if (host == null || !(host.equals(rpId) || host.endsWith("." + rpId))) {
                    throw new SecurityException(origin + " cannot use passkeys of " + rpId);
                }
            }
        }

        byte[] clientDataJson(String type, String challenge) {
            if (browserClientDataHash != null) {
                // The browser keeps its own client data; this field is replaced by it.
                return "{}".getBytes(StandardCharsets.UTF_8);
            }
            return WebAuthn.clientDataJson(type, challenge, origin);
        }

        byte[] clientDataHash(byte[] clientDataJson) {
            return browserClientDataHash != null ? browserClientDataHash : WebAuthn.sha256(clientDataJson);
        }
    }

    private Caller caller(CallingAppInfo app, byte[] clientDataHash) throws Exception {
        String webOrigin = app.getOrigin(privilegedBrowsers());
        if (webOrigin != null) {
            if (webOrigin.endsWith("/")) {
                webOrigin = webOrigin.substring(0, webOrigin.length() - 1);
            }
            return new Caller(webOrigin, clientDataHash, true);
        }
        // An app: its origin is the hash of its signing certificate. Any client data hash it
        // sent is ignored, since only privileged browsers may supply their own client data.
        android.content.pm.Signature[] signers = app.getSigningInfo().getApkContentsSigners();
        if (signers == null || signers.length == 0) {
            throw new SecurityException("Cannot identify the app asking for a passkey");
        }
        return new Caller(WebAuthn.apkOrigin(signers[0].toByteArray()), null, false);
    }

    private String privilegedBrowsers() throws Exception {
        try (InputStream in = getAssets().open(PRIVILEGED_BROWSERS)) {
            return new String(in.readAllBytes(), StandardCharsets.UTF_8);
        }
    }
}
