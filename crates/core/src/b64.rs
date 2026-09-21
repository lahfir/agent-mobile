//! Minimal standard-base64 decoder for screenshot payloads; small enough to
//! own rather than take a dependency for.

use crate::error::Failure;

/// Decode standard base64, stopping at padding and skipping whitespace.
///
/// # Errors
/// Returns [`Failure::Local`] on an invalid character.
pub fn decode(input: &str) -> Result<Vec<u8>, Failure> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc = 0u32;
    let mut nbits = 0u32;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => u32::from(byte - b'A'),
            b'a'..=b'z' => u32::from(byte - b'a') + 26,
            b'0'..=b'9' => u32::from(byte - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => {
                return Err(Failure::local(
                    "invalid base64 in screenshot payload",
                    "report a bug; the driver sent non-base64 bytes",
                ));
            }
        };
        acc = (acc << 6) | value;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push(u8::try_from((acc >> nbits) & 0xFF).unwrap_or_default());
        }
    }
    Ok(out)
}
