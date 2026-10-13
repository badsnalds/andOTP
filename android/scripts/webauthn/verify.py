# Verifies the output of Harness.java with py_webauthn, as a relying party (website) would.
import sys, json
from webauthn import verify_registration_response, verify_authentication_response
from webauthn.helpers import base64url_to_bytes, bytes_to_base64url
reg, auth = open(sys.argv[1]).read().splitlines()
r = verify_registration_response(credential=reg, expected_challenge=b"register-challenge-1234567890",
    expected_rp_id="example.com", expected_origin="https://example.com", require_user_verification=True)
print("registration ok: fmt", r.fmt, "alg", r.credential_public_key[:8].hex(), "sign_count", r.sign_count, "backed_up", r.credential_backed_up, "device_type", r.credential_device_type)
a = verify_authentication_response(credential=auth, expected_challenge=b"auth-challenge-0987654321",
    expected_rp_id="example.com", expected_origin="https://example.com",
    credential_public_key=r.credential_public_key, credential_current_sign_count=0, require_user_verification=True)
print("authentication ok: new_sign_count", a.new_sign_count, "user_verified", a.user_verified)
print("userHandle", json.loads(auth)["response"]["userHandle"])

# A tampered signature must be rejected.
bad = json.loads(auth)
sig = bytearray(base64url_to_bytes(bad["response"]["signature"]))
sig[-1] ^= 1
bad["response"]["signature"] = bytes_to_base64url(bytes(sig))
try:
    verify_authentication_response(credential=json.dumps(bad), expected_challenge=b"auth-challenge-0987654321",
        expected_rp_id="example.com", expected_origin="https://example.com",
        credential_public_key=r.credential_public_key, credential_current_sign_count=0)
except Exception as e:
    print("tampered signature rejected:", type(e).__name__)
else:
    sys.exit("a tampered signature was accepted")
