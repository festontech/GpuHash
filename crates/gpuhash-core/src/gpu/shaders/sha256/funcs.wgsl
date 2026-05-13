// SHA-256 per-algorithm pieces. Depends on `common/match.wgsl` for `targets`,
// `match_buf`, `rotl`, `rotr`, `byteswap`, and `pad_be_block`.
//
// Reference: NIST FIPS 180-4 §6.2.

// 64 round constants K[i] = first 32 bits of the fractional parts of the
// cube roots of the first 64 primes.
var<private> K_SHA256: array<u32, 64> = array<u32, 64>(
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u,
    0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
    0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u,
    0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
    0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu,
    0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u,
    0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
    0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u,
    0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
    0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u,
    0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u,
    0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
    0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u,
    0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u,
);

fn ch(x: u32, y: u32, z: u32) -> u32 {
    return (x & y) ^ ((~x) & z);
}
fn maj(x: u32, y: u32, z: u32) -> u32 {
    return (x & y) ^ (x & z) ^ (y & z);
}
fn big_sigma0(x: u32) -> u32 {
    return rotr(x, 2u) ^ rotr(x, 13u) ^ rotr(x, 22u);
}
fn big_sigma1(x: u32) -> u32 {
    return rotr(x, 6u) ^ rotr(x, 11u) ^ rotr(x, 25u);
}
fn small_sigma0(x: u32) -> u32 {
    return rotr(x, 7u) ^ rotr(x, 18u) ^ (x >> 3u);
}
fn small_sigma1(x: u32) -> u32 {
    return rotr(x, 17u) ^ rotr(x, 19u) ^ (x >> 10u);
}

// Compute SHA-256 over a single 64-byte block already padded by the caller.
// `M_in` holds 16 big-endian message words. Returns the eight state words
// as an 8-element array in BE form (matches host packing).
fn sha256_block(M_in: array<u32, 16>) -> array<u32, 8> {
    // Same naga rule: function-parameter arrays can't be dynamically indexed.
    var M = M_in;

    var W: array<u32, 64>;
    for (var i = 0u; i < 16u; i = i + 1u) {
        W[i] = M[i];
    }
    for (var i = 16u; i < 64u; i = i + 1u) {
        W[i] = small_sigma1(W[i - 2u]) + W[i - 7u]
             + small_sigma0(W[i - 15u]) + W[i - 16u];
    }

    var a: u32 = 0x6a09e667u;
    var b: u32 = 0xbb67ae85u;
    var c: u32 = 0x3c6ef372u;
    var d: u32 = 0xa54ff53au;
    var e: u32 = 0x510e527fu;
    var f: u32 = 0x9b05688cu;
    var g: u32 = 0x1f83d9abu;
    var h: u32 = 0x5be0cd19u;
    let a0 = a; let b0 = b; let c0 = c; let d0 = d;
    let e0 = e; let f0 = f; let g0 = g; let h0 = h;

    for (var t = 0u; t < 64u; t = t + 1u) {
        let t1 = h + big_sigma1(e) + ch(e, f, g) + K_SHA256[t] + W[t];
        let t2 = big_sigma0(a) + maj(a, b, c);
        h = g;
        g = f;
        f = e;
        e = d + t1;
        d = c;
        c = b;
        b = a;
        a = t1 + t2;
    }

    return array<u32, 8>(
        a + a0, b + b0, c + c0, d + d0,
        e + e0, f + f0, g + g0, h + h0,
    );
}

fn scan_targets_sha256(
    h: array<u32, 8>,
    cand_idx: u32,
    num_targets: u32,
    max_matches: u32,
) {
    for (var t = 0u; t < num_targets; t = t + 1u) {
        let base = t * 8u;
        if (h[0] == targets[base]
         && h[1] == targets[base + 1u]
         && h[2] == targets[base + 2u]
         && h[3] == targets[base + 3u]
         && h[4] == targets[base + 4u]
         && h[5] == targets[base + 5u]
         && h[6] == targets[base + 6u]
         && h[7] == targets[base + 7u]) {
            let slot_idx = atomicAdd(&match_buf.count, 1u);
            if (slot_idx < max_matches) {
                match_buf.pairs[slot_idx * 2u]      = cand_idx;
                match_buf.pairs[slot_idx * 2u + 1u] = t;
            }
        }
    }
}
