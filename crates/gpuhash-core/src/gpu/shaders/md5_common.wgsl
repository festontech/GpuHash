// Shared MD5 helpers and result-channel bindings for both the dictionary and
// bruteforce attack kernels.
//
// Conventions:
//   @group(0) @binding(1)  targets (storage, read)
//   @group(0) @binding(2)  match_buf (storage, read_write)
//
// Per-kernel bindings (candidates / mask, params) sit at @binding 0 and
// @binding 3 in the dict/brute-specific .wgsl files that follow this one.

struct MatchBuf {
    count: atomic<u32>,
    _pad:  array<u32, 3>,
    pairs: array<u32>,
}

@group(0) @binding(1) var<storage, read>       targets:   array<u32>;
@group(0) @binding(2) var<storage, read_write> match_buf: MatchBuf;

// MD5 round constants: K[i] = floor(abs(sin(i + 1)) * 2^32) for i in 0..64.
// `var<private>` allows dynamic indexing inside the kernel (naga rejects this
// for `const` arrays).
var<private> K: array<u32, 64> = array<u32, 64>(
    0xd76aa478u, 0xe8c7b756u, 0x242070dbu, 0xc1bdceeeu,
    0xf57c0fafu, 0x4787c62au, 0xa8304613u, 0xfd469501u,
    0x698098d8u, 0x8b44f7afu, 0xffff5bb1u, 0x895cd7beu,
    0x6b901122u, 0xfd987193u, 0xa679438eu, 0x49b40821u,
    0xf61e2562u, 0xc040b340u, 0x265e5a51u, 0xe9b6c7aau,
    0xd62f105du, 0x02441453u, 0xd8a1e681u, 0xe7d3fbc8u,
    0x21e1cde6u, 0xc33707d6u, 0xf4d50d87u, 0x455a14edu,
    0xa9e3e905u, 0xfcefa3f8u, 0x676f02d9u, 0x8d2a4c8au,
    0xfffa3942u, 0x8771f681u, 0x6d9d6122u, 0xfde5380cu,
    0xa4beea44u, 0x4bdecfa9u, 0xf6bb4b60u, 0xbebfbc70u,
    0x289b7ec6u, 0xeaa127fau, 0xd4ef3085u, 0x04881d05u,
    0xd9d4d039u, 0xe6db99e5u, 0x1fa27cf8u, 0xc4ac5665u,
    0xf4292244u, 0x432aff97u, 0xab9423a7u, 0xfc93a039u,
    0x655b59c3u, 0x8f0ccc92u, 0xffeff47du, 0x85845dd1u,
    0x6fa87e4fu, 0xfe2ce6e0u, 0xa3014314u, 0x4e0811a1u,
    0xf7537e82u, 0xbd3af235u, 0x2ad7d2bbu, 0xeb86d391u,
);

// Per-round left-rotate amounts.
var<private> S: array<u32, 64> = array<u32, 64>(
    7u, 12u, 17u, 22u,  7u, 12u, 17u, 22u,  7u, 12u, 17u, 22u,  7u, 12u, 17u, 22u,
    5u,  9u, 14u, 20u,  5u,  9u, 14u, 20u,  5u,  9u, 14u, 20u,  5u,  9u, 14u, 20u,
    4u, 11u, 16u, 23u,  4u, 11u, 16u, 23u,  4u, 11u, 16u, 23u,  4u, 11u, 16u, 23u,
    6u, 10u, 15u, 21u,  6u, 10u, 15u, 21u,  6u, 10u, 15u, 21u,  6u, 10u, 15u, 21u,
);

fn rotl(x: u32, n: u32) -> u32 {
    return (x << n) | (x >> (32u - n));
}

// Compute MD5 over a single 64-byte block. Caller must have already done the
// 0x80 marker and bit-length padding inside `m`. Returns the four state words
// as a vec4 (A, B, C, D).
fn md5_block(m_in: array<u32, 16>) -> vec4<u32> {
    var m = m_in;
    var a: u32 = 0x67452301u;
    var b: u32 = 0xefcdab89u;
    var c: u32 = 0x98badcfeu;
    var d: u32 = 0x10325476u;

    let a0 = a;
    let b0 = b;
    let c0 = c;
    let d0 = d;

    for (var i = 0u; i < 64u; i = i + 1u) {
        var f: u32;
        var g: u32;
        if (i < 16u) {
            f = (b & c) | ((~b) & d);
            g = i;
        } else if (i < 32u) {
            f = (d & b) | ((~d) & c);
            g = (5u * i + 1u) & 15u;
        } else if (i < 48u) {
            f = b ^ c ^ d;
            g = (3u * i + 5u) & 15u;
        } else {
            f = c ^ (b | (~d));
            g = (7u * i) & 15u;
        }
        let temp = d;
        d = c;
        c = b;
        b = b + rotl(a + f + K[i] + m[g], S[i]);
        a = temp;
    }

    return vec4<u32>(a + a0, b + b0, c + c0, d + d0);
}

// Compare a computed digest against the entire target list; on a hit, reserve
// a slot in the match buffer.
fn scan_targets(h: vec4<u32>, cand_idx: u32, num_targets: u32, max_matches: u32) {
    for (var t = 0u; t < num_targets; t = t + 1u) {
        let base = t * 4u;
        if (h.x == targets[base]
         && h.y == targets[base + 1u]
         && h.z == targets[base + 2u]
         && h.w == targets[base + 3u]) {
            let slot_idx = atomicAdd(&match_buf.count, 1u);
            if (slot_idx < max_matches) {
                match_buf.pairs[slot_idx * 2u]      = cand_idx;
                match_buf.pairs[slot_idx * 2u + 1u] = t;
            }
        }
    }
}
