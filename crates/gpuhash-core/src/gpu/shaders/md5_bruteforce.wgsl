// Bruteforce-attack entry point. The host parses the mask, packs one
// `MaskPos` per position, and dispatches one thread per candidate index. Each
// thread synthesizes its own candidate by decomposing
// `base_index + gid.x` across the mask's per-position widths.
//
// Position numbering matches `crate::mask::Mask`: position 0 is the most
// significant (leftmost in the mask string), the last position is the least
// significant.

// One mask position. Mirror of `crate::gpu::buffers::MaskPosGpu`.
//   kind = 0  literal byte; `value` is the byte
//   kind = 1  lowercase a-z; `value` ignored
//   kind = 2  uppercase A-Z; `value` ignored
//   kind = 3  digit    0-9;  `value` ignored
struct MaskPos {
    kind:  u32,
    value: u32,
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

@compute @workgroup_size(64)
fn md5_bruteforce(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }

    let abs_idx = params.base_index + cand_idx;
    let len = params.num_positions;

    var m: array<u32, 16>;

    // Decompose abs_idx across positions (least-significant = last position).
    var remaining = abs_idx;
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
        let shift = (p & 3u) << 3u;
        m[word] = m[word] | (byte_value << shift);
    }

    // 0x80 marker at byte position `len`.
    let word_idx = len >> 2u;
    let byte_off = (len & 3u) << 3u;
    m[word_idx] = m[word_idx] | (0x80u << byte_off);

    // Bit length in the trailing 8 bytes.
    m[14] = len << 3u;
    m[15] = 0u;

    let h = md5_block(m);
    scan_targets(h, cand_idx, params.num_targets, params.max_matches);
}
