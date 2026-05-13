// Pieces every attack kernel reuses, regardless of algorithm or mode:
//   - the match-output channel (bindings 1 and 2)
//   - small u32 bit-twiddling helpers (rotl, byteswap)
//
// The targets buffer is `array<u32>` and is indexed per-algorithm:
// digest_words * target_idx + state_word. The host packs each digest in the
// algorithm's natural endianness (see kernel_spec::Endianness) so the shader's
// comparison can be a flat u32==u32.

struct MatchBuf {
    count: atomic<u32>,
    _pad:  array<u32, 3>,
    pairs: array<u32>,
}

@group(0) @binding(1) var<storage, read>       targets:   array<u32>;
@group(0) @binding(2) var<storage, read_write> match_buf: MatchBuf;

fn rotl(x: u32, n: u32) -> u32 {
    return (x << n) | (x >> (32u - n));
}

fn rotr(x: u32, n: u32) -> u32 {
    return (x >> n) | (x << (32u - n));
}

fn byteswap(x: u32) -> u32 {
    return ((x >> 24u) & 0x000000ffu)
         | ((x >> 8u)  & 0x0000ff00u)
         | ((x << 8u)  & 0x00ff0000u)
         | ((x << 24u) & 0xff000000u);
}

// Single-block big-endian padding shared by SHA-1 and SHA-256: append the 0x80
// marker at byte `len`, zero-fill, then the 64-bit bit-length BE in the last 8
// bytes. The caller has already filled `M[0..(len/4)+1]` with the message bytes
// as big-endian u32 words; bytes beyond `len` in those words are zero.
fn pad_be_block(M_in: array<u32, 16>, len: u32) -> array<u32, 16> {
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
