// MD5 dictionary-attack kernel.
//
// Each invocation:
//   1. Loads its CandidateSlot (one slot per global invocation).
//   2. Builds a 64-byte message block (single-block MD5: assumes len <= 55).
//   3. Runs the 64 MD5 rounds and produces a 4-word state.
//   4. Linear-scans the target hashes and, on a hit, atomically reserves a slot
//      in the match buffer.
//
// Single-block constraint: a 64-byte MD5 block must accommodate the input bytes,
// a trailing 0x80, zero padding, and an 8-byte bit-length suffix. That caps the
// input length at 55 bytes. The Rust host enforces this before dispatching.
// Multi-block input lands with brute-force candidate generation in Phase 4 or
// with longer-password support in a later phase.

struct CandidateSlot {
    len: u32,
    bytes: array<u32, 14>,  // up to 56 bytes of input, packed little-endian
}

struct Params {
    num_candidates: u32,
    num_targets:    u32,
    max_matches:    u32,
    _pad:           u32,
}

struct MatchBuf {
    count: atomic<u32>,
    _pad:  array<u32, 3>,   // align `pairs` to 16 bytes for clarity
    pairs: array<u32>,      // flat: [cand0, tgt0, cand1, tgt1, ...]
}

@group(0) @binding(0) var<storage, read>       candidates: array<CandidateSlot>;
@group(0) @binding(1) var<storage, read>       targets:    array<u32>;   // 4 words per target
@group(0) @binding(2) var<storage, read_write> match_buf:  MatchBuf;
@group(0) @binding(3) var<uniform>             params:     Params;

// MD5 round constants: K[i] = floor(abs(sin(i + 1)) * 2^32) for i in 0..64.
// Held in `var<private>` rather than `const` because naga rejects dynamic
// indexing of const-array values; var<private> with a const initializer is the
// idiomatic workaround for "read-only table the kernel walks with `K[i]`".
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

// Per-round left-rotate amounts. Same `var<private>` rationale as K.
var<private> S: array<u32, 64> = array<u32, 64>(
    7u, 12u, 17u, 22u,  7u, 12u, 17u, 22u,  7u, 12u, 17u, 22u,  7u, 12u, 17u, 22u,
    5u,  9u, 14u, 20u,  5u,  9u, 14u, 20u,  5u,  9u, 14u, 20u,  5u,  9u, 14u, 20u,
    4u, 11u, 16u, 23u,  4u, 11u, 16u, 23u,  4u, 11u, 16u, 23u,  4u, 11u, 16u, 23u,
    6u, 10u, 15u, 21u,  6u, 10u, 15u, 21u,  6u, 10u, 15u, 21u,  6u, 10u, 15u, 21u,
);

fn rotl(x: u32, n: u32) -> u32 {
    return (x << n) | (x >> (32u - n));
}

@compute @workgroup_size(64)
fn md5_attack(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }

    // Copy the candidate slot into a function-scoped `var` so the inner
    // `bytes[i]` access is allowed to use a dynamic index. WGSL forbids
    // dynamic indexing of array values held in `let` bindings.
    var slot = candidates[cand_idx];
    let len = slot.len;

    // Build the 16-word message block. Host has already packed input bytes
    // little-endian into bytes[0..14] and zeroed any trailing bytes.
    var m: array<u32, 16>;
    for (var i = 0u; i < 14u; i = i + 1u) {
        m[i] = slot.bytes[i];
    }

    // Append the 0x80 marker byte at byte position `len`.
    // Word index = len / 4; bit offset within word = (len % 4) * 8.
    let word_idx = len >> 2u;
    let byte_off = (len & 3u) << 3u;
    m[word_idx] = m[word_idx] | (0x80u << byte_off);

    // Bit length goes in the last 8 bytes of the block.
    m[14] = len << 3u;   // len * 8, low 32 bits
    m[15] = 0u;          // high 32 bits — len is in bytes so always fits in u32

    // MD5 initial state.
    var a: u32 = 0x67452301u;
    var b: u32 = 0xefcdab89u;
    var c: u32 = 0x98badcfeu;
    var d: u32 = 0x10325476u;

    let a0 = a;
    let b0 = b;
    let c0 = c;
    let d0 = d;

    // 64 rounds, four 16-round groups with different mixing function f and
    // message-schedule index g.
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

    a = a + a0;
    b = b + b0;
    c = c + c0;
    d = d + d0;

    // Scan target list. On a hit, atomically reserve a match slot.
    for (var t = 0u; t < params.num_targets; t = t + 1u) {
        let base = t * 4u;
        if (a == targets[base]
         && b == targets[base + 1u]
         && c == targets[base + 2u]
         && d == targets[base + 3u]) {
            let slot_idx = atomicAdd(&match_buf.count, 1u);
            if (slot_idx < params.max_matches) {
                match_buf.pairs[slot_idx * 2u]      = cand_idx;
                match_buf.pairs[slot_idx * 2u + 1u] = t;
            }
        }
    }
}
