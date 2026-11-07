//! Sticker files on disk: saved stickers and sticker packs.

/// The file's content hash, the name stickers carry on disk so the same
/// picture is one file.
pub fn content_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    // Write lowercase hex straight into a stack buffer instead of 32 per-byte
    // `format!` allocations (same shape as `read_api::hash_token`). Output is
    // identical lowercase hex.
    let mut out = [0u8; 64];
    for (i, byte) in digest.iter().enumerate() {
        out[i * 2] = HEX[(byte >> 4) as usize];
        out[i * 2 + 1] = HEX[(byte & 0x0F) as usize];
    }
    String::from_utf8(out.to_vec()).expect("hex digits are valid UTF-8")
}
