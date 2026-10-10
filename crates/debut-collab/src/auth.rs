//! Joining a shared session (NFR-13). The host hands out invite codes, one
//! per role; the server never trusts a role a client claims. On connect it
//! sends a fresh nonce, the client answers with HMAC-SHA256(code, nonce),
//! and the role is the one whose code verifies. The code itself never
//! crosses the network, and a captured answer is useless for another nonce.

use crate::Role;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Codes for the roles a session admits. A role without a code is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub editor: String,
    pub reviewer: Option<String>,
}

impl Invite {
    /// Codes from 20 random bytes each (from the caller: this crate makes
    /// no OS calls): `edit-xxxx-xxxx-xxxx-xxxx`, `review-...`.
    pub fn from_random(editor: [u8; 10], reviewer: [u8; 10]) -> Self {
        Self {
            editor: format!("edit-{}", code(&editor)),
            reviewer: Some(format!("review-{}", code(&reviewer))),
        }
    }

    /// The role whose code produced `proof` for `nonce`, if any.
    pub fn role_for(&self, nonce: &str, proof: &str) -> Option<Role> {
        if verify(&self.editor, nonce, proof) {
            return Some(Role::Editor);
        }
        match &self.reviewer {
            Some(code) if verify(code, nonce, proof) => Some(Role::Reviewer),
            _ => None,
        }
    }
}

/// Base32 (Crockford, no confusable letters) in groups of four.
fn code(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = String::new();
    let mut chars = 0usize;
    for &b in bytes {
        bits = (bits << 8) | b as u32;
        n += 8;
        while n >= 5 {
            n -= 5;
            if chars > 0 && chars.is_multiple_of(4) {
                out.push('-');
            }
            out.push(ALPHABET[((bits >> n) & 31) as usize] as char);
            chars += 1;
        }
    }
    out
}

fn mac(code: &str, nonce: &str) -> HmacSha256 {
    let mut m = HmacSha256::new_from_slice(code.trim().as_bytes()).expect("any key length");
    m.update(b"debut-collab-v1:");
    m.update(nonce.as_bytes());
    m
}

/// The answer to `nonce` for someone holding `code` (hex).
pub fn proof(code: &str, nonce: &str) -> String {
    mac(code, nonce)
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Constant-time check of `proof` against `code` and `nonce`.
pub fn verify(code: &str, nonce: &str, proof: &str) -> bool {
    let bytes: Option<Vec<u8>> = (0..proof.len())
        .step_by(2)
        .map(|i| {
            proof
                .get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect();
    match bytes {
        Some(b) if b.len() == 32 => mac(code, nonce).verify_slice(&b).is_ok(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_the_rfc_4231_vector() {
        // RFC 4231 test case 2, through the same primitive.
        let mut m = HmacSha256::new_from_slice(b"Jefe").unwrap();
        m.update(b"what do ya want for nothing?");
        let hex: String = m
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn roles_come_from_the_code_not_the_client() {
        let invite = Invite::from_random([1; 10], [2; 10]);
        assert!(invite.editor.starts_with("edit-"));
        assert_eq!(invite.editor.len(), "edit-".len() + 16 + 3);
        let nonce = "n1";
        let ed = proof(&invite.editor, nonce);
        let rv = proof(invite.reviewer.as_ref().unwrap(), nonce);
        assert_eq!(invite.role_for(nonce, &ed), Some(Role::Editor));
        assert_eq!(invite.role_for(nonce, &rv), Some(Role::Reviewer));
        assert_eq!(invite.role_for(nonce, &proof("guess", nonce)), None);
        // A captured answer does not work for another nonce.
        assert_eq!(invite.role_for("n2", &ed), None);
        // Garbage is refused, not a panic.
        for bad in ["", "zz", "abc", &"0".repeat(64)] {
            assert_eq!(invite.role_for(nonce, bad), None);
        }
        // Surrounding spaces from copy-paste do not matter.
        assert_eq!(
            invite.role_for(nonce, &proof(&format!(" {} ", invite.editor), nonce)),
            Some(Role::Editor)
        );
        let closed = Invite {
            reviewer: None,
            ..invite.clone()
        };
        assert_eq!(closed.role_for(nonce, &rv), None);
    }
}
