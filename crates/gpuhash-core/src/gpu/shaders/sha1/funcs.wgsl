// SHA-1 per-algorithm pieces. Depends on `match_common.wgsl` for `targets`,
// `match_buf`, `rotl`, and `byteswap`.

var<private> K_SHA1: array<u32, 4> = array<u32, 4>(
    0x5a827999u, 0x6ed9eba1u, 0x8f1bbcdcu, 0xca62c1d6u,
);

// Compute SHA-1 over a single 64-byte block already padded by the caller.
// `M_in` holds 16 big-endian message words. Returns the five state words
// as a 5-element array (a, b, c, d, e) in BE form so the host can compare
// them directly against `targets[base..base+5]`.
fn sha1_block(M_in: array<u32, 16>) -> array<u32, 5> {
    // Same naga rule as md5_block: function-parameter array values can't be
    // dynamically indexed. Copy into a `var` first.
    var M = M_in;

    var W: array<u32, 80>;
    for (var i = 0u; i < 16u; i = i + 1u) {
        W[i] = M[i];
    }
    for (var i = 16u; i < 80u; i = i + 1u) {
        W[i] = rotl(W[i - 3u] ^ W[i - 8u] ^ W[i - 14u] ^ W[i - 16u], 1u);
    }

    var a: u32 = 0x67452301u;
    var b: u32 = 0xefcdab89u;
    var c: u32 = 0x98badcfeu;
    var d: u32 = 0x10325476u;
    var e: u32 = 0xc3d2e1f0u;
    let a0 = a;
    let b0 = b;
    let c0 = c;
    let d0 = d;
    let e0 = e;

    for (var t = 0u; t < 80u; t = t + 1u) {
        var f: u32;
        var k: u32;
        if (t < 20u) {
            f = (b & c) | ((~b) & d);
            k = K_SHA1[0];
        } else if (t < 40u) {
            f = b ^ c ^ d;
            k = K_SHA1[1];
        } else if (t < 60u) {
            f = (b & c) | (b & d) | (c & d);
            k = K_SHA1[2];
        } else {
            f = b ^ c ^ d;
            k = K_SHA1[3];
        }
        let temp = rotl(a, 5u) + f + e + k + W[t];
        e = d;
        d = c;
        c = rotl(b, 30u);
        b = a;
        a = temp;
    }

    return array<u32, 5>(a + a0, b + b0, c + c0, d + d0, e + e0);
}

// Big-endian padding: 0x80 marker at byte `len`, then zero-pad, then 64-bit
// bit-length BE in the last 8 bytes.
fn sha1_pad_block(M_in: array<u32, 16>, len: u32) -> array<u32, 16> {
    var M = M_in;
    let word_idx = len >> 2u;
    let byte_in_word = len & 3u;
    // BE: byte 0 of a word lives in the high-order byte (shift 24).
    let shift = (3u - byte_in_word) * 8u;
    M[word_idx] = M[word_idx] | (0x80u << shift);

    M[14] = 0u;          // upper 32 bits of bit-length (0 for len < 2^29)
    M[15] = len << 3u;   // lower 32 bits, already in BE u32 representation
    return M;
}

fn scan_targets_sha1(
    h: array<u32, 5>,
    cand_idx: u32,
    num_targets: u32,
    max_matches: u32,
) {
    for (var t = 0u; t < num_targets; t = t + 1u) {
        let base = t * 5u;
        if (h[0] == targets[base]
         && h[1] == targets[base + 1u]
         && h[2] == targets[base + 2u]
         && h[3] == targets[base + 3u]
         && h[4] == targets[base + 4u]) {
            let slot_idx = atomicAdd(&match_buf.count, 1u);
            if (slot_idx < max_matches) {
                match_buf.pairs[slot_idx * 2u]      = cand_idx;
                match_buf.pairs[slot_idx * 2u + 1u] = t;
            }
        }
    }
}
