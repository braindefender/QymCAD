//! Self-contained standard SHA-256 (FIPS 180-4) implementation and QymCAD bundle trailer verification.

/// Initial hash state constants (first 32 bits of the fractional parts of the square roots of the first 8 primes 2..19).
const H0: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

/// Round constants (first 32 bits of the fractional parts of the cube roots of the first 64 primes 2..311).
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Compute standard SHA-256 digest of arbitrary input data.
pub fn compute_sha256(data: &[u8]) -> [u8; 32] {
    let mut h = H0;

    // Pre-processing: padding
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 64);
    padded.extend_from_slice(data);
    padded.push(0x80);

    while (padded.len() % 64) != 56 {
        padded.push(0x00);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 64-byte chunk
    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, slot) in w.iter_mut().take(16).enumerate() {
            *slot = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut h_var = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h_var.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h_var = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(h_var);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// 4-byte magic signature placed at the very end of a verified QymCAD bundle file.
pub const QICONS_TRAILER_MAGIC: &[u8; 4] = b"QCAD";

/// Current version of the trailer format.
pub const QICONS_TRAILER_VERSION: u32 = 1;

/// Total size of the trailer in bytes: 32 bytes SHA-256 + 4 bytes version + 4 bytes magic.
pub const QICONS_TRAILER_LEN: usize = 40;

/// Append the 40-byte QymCAD verification trailer to raw ZIP bytes.
pub fn append_qicons_trailer(zip_bytes: &mut Vec<u8>) {
    let hash = compute_sha256(zip_bytes);
    zip_bytes.extend_from_slice(&hash);
    zip_bytes.extend_from_slice(&QICONS_TRAILER_VERSION.to_le_bytes());
    zip_bytes.extend_from_slice(QICONS_TRAILER_MAGIC);
}

/// Result of checking trailer signature on an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailerCheck {
    /// Authentic QymCAD bundle with matching cryptographic hash.
    Verified,
    /// Magic signature was present, but file content was modified or corrupted.
    Tampered,
    /// Normal ZIP archive without trailer (e.g. community zip or manually renamed).
    Unsigned,
}

/// Inspect the trailing bytes of a file to check if it has a valid QymCAD bundle signature.
pub fn verify_qicons_trailer(file_bytes: &[u8]) -> TrailerCheck {
    if file_bytes.len() < QICONS_TRAILER_LEN {
        return TrailerCheck::Unsigned;
    }

    let len = file_bytes.len();
    let magic = &file_bytes[len - 4..];
    if magic != QICONS_TRAILER_MAGIC {
        return TrailerCheck::Unsigned;
    }

    let version_bytes: [u8; 4] = file_bytes[len - 8..len - 4].try_into().unwrap();
    let version = u32::from_le_bytes(version_bytes);
    if version != QICONS_TRAILER_VERSION {
        return TrailerCheck::Tampered;
    }

    let expected_hash = &file_bytes[len - 40..len - 8];
    let actual_hash = compute_sha256(&file_bytes[..len - 40]);

    if expected_hash == actual_hash {
        TrailerCheck::Verified
    } else {
        TrailerCheck::Tampered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_nist_vectors() {
        // Empty string
        assert_eq!(
            compute_sha256(b""),
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52,
                0xb8, 0x55,
            ]
        );

        // "abc"
        assert_eq!(
            compute_sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
                0x15, 0xad,
            ]
        );
    }

    #[test]
    fn test_qicons_trailer_verification() {
        let mut sample = b"PK\x03\x04dummy_zip_content".to_vec();

        // Initially unsigned
        assert_eq!(verify_qicons_trailer(&sample), TrailerCheck::Unsigned);

        // Sign it with trailer
        append_qicons_trailer(&mut sample);
        assert_eq!(verify_qicons_trailer(&sample), TrailerCheck::Verified);

        // Modify 1 byte in the content
        sample[5] ^= 0xFF;
        assert_eq!(verify_qicons_trailer(&sample), TrailerCheck::Tampered);
    }
}
