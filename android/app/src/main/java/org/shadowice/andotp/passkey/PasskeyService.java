package org.shadowice.andotp.passkey;

import android.app.PendingIntent;
import android.content.Intent;
import android.os.CancellationSignal;
import android.os.OutcomeReceiver;

import androidx.annotation.RequiresApi;
import androidx.credentials.exceptions.ClearCredentialException;
import androidx.credentials.exceptions.CreateCredentialException;
import androidx.credentials.exceptions.CreateCredentialUnknownException;
import androidx.credentials.exceptions.GetCredentialException;
import androidx.credentials.exceptions.GetCredentialUnknownException;
import androidx.credentials.provider.BeginCreateCredentialRequest;
import androidx.credentials.provider.BeginCreateCredentialResponse;
import androidx.credentials.provider.BeginCreatePublicKeyCredentialRequest;
import androidx.credentials.provider.BeginGetCredentialOption;
import androidx.credentials.provider.BeginGetCredentialRequest;
import androidx.credentials.provider.BeginGetCredentialResponse;
import androidx.credentials.provider.BeginGetPublicKeyCredentialOption;
import androidx.credentials.provider.CreateEntry;
import androidx.credentials.provider.CredentialProviderService;
import androidx.credentials.provider.ProviderClearCredentialStateRequest;
import androidx.credentials.provider.PublicKeyCredentialEntry;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/**
 * Makes andOTP a passkey provider for the Android Credential Manager (Android 14+). The
 * system calls this service when an app or browser wants to create or use a passkey; each
 * entry offered here opens {@link PasskeyActivity}, which asks for the fingerprint (or screen
 * lock) and does the actual WebAuthn work.
 */
@RequiresApi(34)
public class PasskeyService extends CredentialProviderService {
    static final String EXTRA_CREDENTIAL_ID = "org.shadowice.andotp.passkey.CREDENTIAL_ID";
    static final String ACTION_CREATE = "org.shadowice.andotp.passkey.CREATE";
    static final String ACTION_GET = "org.shadowice.andotp.passkey.GET";

    private static int nextRequestCode = 1;

    @Override
    public void onBeginCreateCredentialRequest(BeginCreateCredentialRequest request,
                                               CancellationSignal cancellationSignal,
                                               OutcomeReceiver<BeginCreateCredentialResponse, CreateCredentialException> callback) {
        if (!(request instanceof BeginCreatePublicKeyCredentialRequest)) {
            callback.onError(new CreateCredentialUnknownException("andOTP only stores passkeys"));
            return;
        }
        int count = PasskeyStore.list(this).size();
        CreateEntry entry = new CreateEntry.Builder("andOTP", pendingIntent(ACTION_CREATE, null))
                .setDescription("Passkey saved on this phone only, unlocked with your fingerprint or screen lock")
                .setPublicKeyCredentialCount(count)
                .build();
        callback.onResult(new BeginCreateCredentialResponse.Builder().addCreateEntry(entry).build());
    }

    @Override
    public void onBeginGetCredentialRequest(BeginGetCredentialRequest request,
                                            CancellationSignal cancellationSignal,
                                            OutcomeReceiver<BeginGetCredentialResponse, GetCredentialException> callback) {
        try {
            BeginGetCredentialResponse.Builder response = new BeginGetCredentialResponse.Builder();
            for (BeginGetCredentialOption option : request.getBeginGetCredentialOptions()) {
                if (!(option instanceof BeginGetPublicKeyCredentialOption)) {
                    continue;
                }
                BeginGetPublicKeyCredentialOption publicKeyOption = (BeginGetPublicKeyCredentialOption) option;
                JSONObject json = new JSONObject(publicKeyOption.getRequestJson());
                String rpId = json.optString("rpId");
                for (PasskeyStore.Passkey passkey : PasskeyStore.forRp(this, rpId, allowedIds(json))) {
                    String user = passkey.userName.isEmpty() ? passkey.displayName : passkey.userName;
                    response.addCredentialEntry(new PublicKeyCredentialEntry.Builder(this, user,
                            pendingIntent(ACTION_GET, passkey.credentialId), publicKeyOption)
                            .setDisplayName(passkey.displayName.isEmpty() ? null : passkey.displayName)
                            .build());
                }
            }
            callback.onResult(response.build());
        } catch (Exception e) {
            callback.onError(new GetCredentialUnknownException(e.getMessage()));
        }
    }

    @Override
    public void onClearCredentialStateRequest(ProviderClearCredentialStateRequest request,
                                              CancellationSignal cancellationSignal,
                                              OutcomeReceiver<Void, ClearCredentialException> callback) {
        // No sessions are kept, so there is nothing to clear.
        callback.onResult(null);
    }

    /** The credential ids listed in "allowCredentials" (empty = any passkey for the site). */
    static List<String> allowedIds(JSONObject request) {
        List<String> ids = new ArrayList<>();
        JSONArray allow = request.optJSONArray("allowCredentials");
        if (allow != null) {
            for (int i = 0; i < allow.length(); i++) {
                JSONObject credential = allow.optJSONObject(i);
                if (credential != null) {
                    ids.add(credential.optString("id"));
                }
            }
        }
        return ids;
    }

    private PendingIntent pendingIntent(String action, String credentialId) {
        Intent intent = new Intent(this, PasskeyActivity.class).setAction(action);
        if (credentialId != null) {
            intent.putExtra(EXTRA_CREDENTIAL_ID, credentialId);
        }
        int requestCode;
        synchronized (PasskeyService.class) {
            requestCode = nextRequestCode++;
        }
        // Mutable: the Credential Manager adds the actual request to the intent.
        return PendingIntent.getActivity(this, requestCode, intent,
                PendingIntent.FLAG_MUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }
}
