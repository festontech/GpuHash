// Inputs, parameters, and shared candidate synthesis for every algorithm's
// bruteforce-mode entry point.
//
// `synthesize_candidate_le` does the mask → candidate-bytes decomposition once.
// It writes the candidate bytes little-endian into a `[u32; 16]` (positions
// beyond `len` are zero). LE-byte-order algorithms (MD5) use the result
// directly; big-endian algorithms (SHA-1, SHA-256) byteswap each u32 first.

struct MaskPos {
    kind:  u32,   // 0=literal, 1=lower a-z, 2=upper A-Z, 3=digit 0-9
    value: u32,   // literal byte when kind==0
}

struct Params {
    num_positions:  u32,
    num_candidates: u32,
    num_targets:    u32,
    max_matches:    u32,
    base_index:     u32,
    _pad0:          u32,
    _pad1:          u32,
    _pad2:          u32,
}

@group(0) @binding(0) var<storage, read> mask:   array<MaskPos>;
@group(0) @binding(3) var<uniform>       params: Params;

// Decompose `abs_idx` against the mask and write the resulting candidate bytes
// little-endian into a 16-word block. Positions beyond `len` are 0.
fn synthesize_candidate_le(abs_idx: u32, len: u32) -> array<u32, 16> {
    var bytes_le: array<u32, 16>;
    var remaining = abs_idx;
    // Position numbering matches `crate::mask::Mask`: position 0 is most
    // significant (leftmost in the mask string), so iterate from right to left.
    for (var p_rev = 0u; p_rev < len; p_rev = p_rev + 1u) {
        let p = len - 1u - p_rev;
        let pos = mask[p];
        var byte_value: u32;
        switch pos.kind {
            case 0u: {
                byte_value = pos.value & 0xffu;
            }
            case 1u: {  // lower
                let c = remaining % 26u;
                remaining = remaining / 26u;
                byte_value = 0x61u + c;  // 'a' + c
            }
            case 2u: {  // upper
                let c = remaining % 26u;
                remaining = remaining / 26u;
                byte_value = 0x41u + c;  // 'A' + c
            }
            case 3u: {  // digit
                let c = remaining % 10u;
                remaining = remaining / 10u;
                byte_value = 0x30u + c;  // '0' + c
            }
            default: {
                byte_value = 0u;
            }
        }
        let word = p >> 2u;
        let shift = (p & 3u) << 3u;  // LE: byte 0 of a word at shift 0
        bytes_le[word] = bytes_le[word] | (byte_value << shift);
    }
    return bytes_le;
}
