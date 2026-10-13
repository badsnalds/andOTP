import org.shadowice.andotp.passkey.WebAuthn;
import java.security.*;
import java.security.interfaces.ECPublicKey;
import java.security.spec.ECGenParameterSpec;
/** Produces a registration and a sign-in with WebAuthn.java and a software key; verify.py checks them. */
public class Harness {
    public static void main(String[] a) throws Exception {
        String rp = "example.com", origin = "https://example.com";
        KeyPairGenerator g = KeyPairGenerator.getInstance("EC");
        g.initialize(new ECGenParameterSpec("secp256r1"));
        KeyPair kp = g.generateKeyPair();
        byte[] credId = new byte[16]; new SecureRandom().nextBytes(credId);
        byte[] user = "user-123".getBytes();
        String regChallenge = WebAuthn.b64url("register-challenge-1234567890".getBytes());
        byte[] cd = WebAuthn.clientDataJson("webauthn.create", regChallenge, origin);
        byte[] ad = WebAuthn.registrationAuthData(rp, credId, (ECPublicKey) kp.getPublic());
        System.out.println(WebAuthn.registrationResponse(credId, cd, ad, (ECPublicKey) kp.getPublic()));
        String authChallenge = WebAuthn.b64url("auth-challenge-0987654321".getBytes());
        byte[] cd2 = WebAuthn.clientDataJson("webauthn.get", authChallenge, origin);
        byte[] ad2 = WebAuthn.assertionAuthData(rp);
        Signature s = Signature.getInstance("SHA256withECDSA");
        s.initSign(kp.getPrivate());
        s.update(WebAuthn.signedData(ad2, WebAuthn.sha256(cd2)));
        System.out.println(WebAuthn.authenticationResponse(credId, cd2, ad2, s.sign(), user));
    }
}
