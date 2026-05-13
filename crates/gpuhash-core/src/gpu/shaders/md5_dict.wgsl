// Dictionary-attack entry point. The host packs candidates into 60-byte slots
// (little-endian bytes packed into 14 u32 words) and dispatches one thread per
// slot. Single-block MD5 only — host enforces candidate length <= 55 bytes.

struct CandidateSlot {
    len:   u32,
    bytes: array<u32, 14>,  // up to 56 bytes of input
}

struct Params {
    num_candidates: u32,
    num_targets:    u32,
    max_matches:    u32,
    _pad:           u32,
}

@group(0) @binding(0) var<storage, read> candidates: array<CandidateSlot>;
@group(0) @binding(3) var<uniform>       params:     Params;

@compute @workgroup_size(64)
fn md5_attack(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }

    // `var` so the dynamic index `slot.bytes[i]` inside the loop is allowed.
    var slot = candidates[cand_idx];
    let len = slot.len;

    var m: array<u32, 16>;
    for (var i = 0u; i < 14u; i = i + 1u) {
        m[i] = slot.bytes[i];
    }

    let word_idx = len >> 2u;
    let byte_off = (len & 3u) << 3u;
    m[word_idx] = m[word_idx] | (0x80u << byte_off);

    m[14] = len << 3u;
    m[15] = 0u;

    let h = md5_block(m);
    scan_targets(h, cand_idx, params.num_targets, params.max_matches);
}
