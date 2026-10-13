package org.shadowice.andotp.passkey;

import java.io.ByteArrayOutputStream;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.security.interfaces.ECPublicKey;
import java.util.Base64;

import androidx.annotation.RequiresApi;

/**
 * The WebAuthn byte formats a passkey provider produces: authenticator data, the COSE public
 * key, a "none" attestation object, client data and the JSON responses the Credential Manager
 * hands back to the browser or app. Pure Java, so it can be unit tested off the device.
 *
 * <p>andOTP passkeys are device-bound (backup eligible = false), use ES256 and report a
 * signature counter of 0, which WebAuthn allows for authenticators without a counter.
 */
@RequiresApi(34)
public final class WebAuthn {
    /** COSE algorithm identifier of ES256 (ECDSA with P-256 and SHA-256). */
    public static final int ES256 = -7;

    // Authenticator data flags.
    static final int FLAG_USER_PRESENT = 0x01;
    static final int FLAG_USER_VERIFIED = 0x04;
    static final int FLAG_ATTESTED_DATA = 0x40;

    /** All zero: no attestation, so no model identifier either. */
    private static final byte[] AAGUID = new byte[16];

    private WebAuthn() {}

    private static void write(ByteArrayOutputStream out, byte[] data) {
        out.write(data, 0, data.length);
    }

    public static String b64url(byte[] data) {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(data);
    }

    public static byte[] fromB64url(String text) {
        return Base64.getUrlDecoder().decode(text);
    }

    public static byte[] sha256(byte[] data) {
        try {
            return MessageDigest.getInstance("SHA-256").digest(data);
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    /** The "android:apk-key-hash:..." origin of a native app, from its signing certificate. */
    public static String apkOrigin(byte[] signingCertificate) {
        return "android:apk-key-hash:" + b64url(sha256(signingCertificate));
    }

    /** Client data for callers that do not hash it themselves (native apps). */
    public static byte[] clientDataJson(String type, String challengeB64url, String origin) {
        String json = "{\"type\":" + quote(type)
                + ",\"challenge\":" + quote(challengeB64url)
                + ",\"origin\":" + quote(origin)
                + ",\"crossOrigin\":false}";
        return json.getBytes(StandardCharsets.UTF_8);
    }

    /** Authenticator data for a new credential (attested credential data included). */
    public static byte[] registrationAuthData(String rpId, byte[] credentialId, ECPublicKey key) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        write(out, sha256(rpId.getBytes(StandardCharsets.UTF_8)));
        out.write(FLAG_USER_PRESENT | FLAG_USER_VERIFIED | FLAG_ATTESTED_DATA);
        write(out, new byte[4]); // signature counter
        write(out, AAGUID);
        out.write((credentialId.length >> 8) & 0xff);
        out.write(credentialId.length & 0xff);
        write(out, credentialId);
        write(out, coseKey(key));
        return out.toByteArray();
    }

    /** Authenticator data for a sign-in. */
    public static byte[] assertionAuthData(String rpId) {
        ByteBuffer buffer = ByteBuffer.allocate(37);
        buffer.put(sha256(rpId.getBytes(StandardCharsets.UTF_8)));
        buffer.put((byte) (FLAG_USER_PRESENT | FLAG_USER_VERIFIED));
        buffer.putInt(0); // signature counter
        return buffer.array();
    }

    /** The bytes the credential's private key signs for a sign-in: authData || clientDataHash. */
    public static byte[] signedData(byte[] authData, byte[] clientDataHash) {
        byte[] data = new byte[authData.length + clientDataHash.length];
        System.arraycopy(authData, 0, data, 0, authData.length);
        System.arraycopy(clientDataHash, 0, data, authData.length, clientDataHash.length);
        return data;
    }

    /** The public key as a COSE_Key map: {1: 2 (EC2), 3: -7 (ES256), -1: 1 (P-256), -2: x, -3: y}. */
    public static byte[] coseKey(ECPublicKey key) {
        Cbor cbor = new Cbor();
        cbor.mapHeader(5);
        cbor.integer(1).integer(2);
        cbor.integer(3).integer(ES256);
        cbor.integer(-1).integer(1);
        cbor.integer(-2).bytes(unsigned32(key.getW().getAffineX()));
        cbor.integer(-3).bytes(unsigned32(key.getW().getAffineY()));
        return cbor.toByteArray();
    }

    /** {"fmt": "none", "attStmt": {}, "authData": ...} */
    public static byte[] noneAttestation(byte[] authData) {
        Cbor cbor = new Cbor();
        cbor.mapHeader(3);
        cbor.text("fmt").text("none");
        cbor.text("attStmt").mapHeader(0);
        cbor.text("authData").bytes(authData);
        return cbor.toByteArray();
    }

    /** RegistrationResponseJSON, as returned by navigator.credentials.create(). */
    public static String registrationResponse(byte[] credentialId, byte[] clientDataJson, byte[] authData,
                                              ECPublicKey key) {
        String id = quote(b64url(credentialId));
        return "{\"id\":" + id
                + ",\"rawId\":" + id
                + ",\"type\":\"public-key\""
                + ",\"authenticatorAttachment\":\"platform\""
                + ",\"response\":{"
                + "\"clientDataJSON\":" + quote(b64url(clientDataJson))
                + ",\"attestationObject\":" + quote(b64url(noneAttestation(authData)))
                + ",\"authenticatorData\":" + quote(b64url(authData))
                + ",\"publicKey\":" + quote(b64url(key.getEncoded()))
                + ",\"publicKeyAlgorithm\":" + ES256
                + ",\"transports\":[\"internal\",\"hybrid\"]}"
                + ",\"clientExtensionResults\":{\"credProps\":{\"rk\":true}}}";
    }

    /** AuthenticationResponseJSON, as returned by navigator.credentials.get(). */
    public static String authenticationResponse(byte[] credentialId, byte[] clientDataJson, byte[] authData,
                                                byte[] signature, byte[] userHandle) {
        String id = quote(b64url(credentialId));
        return "{\"id\":" + id
                + ",\"rawId\":" + id
                + ",\"type\":\"public-key\""
                + ",\"authenticatorAttachment\":\"platform\""
                + ",\"response\":{"
                + "\"clientDataJSON\":" + quote(b64url(clientDataJson))
                + ",\"authenticatorData\":" + quote(b64url(authData))
                + ",\"signature\":" + quote(b64url(signature))
                + ",\"userHandle\":" + quote(b64url(userHandle)) + "}"
                + ",\"clientExtensionResults\":{}}";
    }

    /** A JSON string literal. */
    static String quote(String text) {
        StringBuilder out = new StringBuilder(text.length() + 2).append('"');
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            switch (c) {
                case '"': out.append("\\\""); break;
                case '\\': out.append("\\\\"); break;
                case '\n': out.append("\\n"); break;
                case '\r': out.append("\\r"); break;
                case '\t': out.append("\\t"); break;
                default:
                    if (c < 0x20) {
                        out.append(String.format("\\u%04x", (int) c));
                    } else {
                        out.append(c);
                    }
            }
        }
        return out.append('"').toString();
    }

    /** A P-256 coordinate as exactly 32 big-endian bytes. */
    static byte[] unsigned32(BigInteger value) {
        byte[] raw = value.toByteArray();
        byte[] out = new byte[32];
        int copy = Math.min(raw.length, 32);
        System.arraycopy(raw, raw.length - copy, out, 32 - copy, copy);
        return out;
    }

    /** The small subset of CBOR (RFC 8949) needed here, in canonical form. */
    static final class Cbor {
        private final ByteArrayOutputStream out = new ByteArrayOutputStream();

        Cbor integer(long value) {
            if (value >= 0) {
                head(0, value);
            } else {
                head(1, -1 - value);
            }
            return this;
        }

        Cbor bytes(byte[] value) {
            head(2, value.length);
            write(out, value);
            return this;
        }

        Cbor text(String value) {
            byte[] utf8 = value.getBytes(StandardCharsets.UTF_8);
            head(3, utf8.length);
            write(out, utf8);
            return this;
        }

        Cbor mapHeader(int entries) {
            head(5, entries);
            return this;
        }

        private void head(int major, long value) {
            int type = major << 5;
            if (value < 24) {
                out.write(type | (int) value);
            } else if (value < 0x100) {
                out.write(type | 24);
                out.write((int) value);
            } else if (value < 0x10000) {
                out.write(type | 25);
                out.write((int) (value >> 8));
                out.write((int) value & 0xff);
            } else {
                out.write(type | 26);
                for (int shift = 24; shift >= 0; shift -= 8) {
                    out.write((int) (value >> shift) & 0xff);
                }
            }
        }

        byte[] toByteArray() {
            return out.toByteArray();
        }
    }
}
