// SHA-1 dictionary entry point. Depends on:
//   match_common.wgsl  — targets, match_buf, rotl, byteswap
//   dict_common.wgsl   — CandidateSlot, Params, candidates, params
//   sha1_funcs.wgsl    — sha1_block, sha1_pad_block, scan_targets_sha1

@compute @workgroup_size(64)
fn sha1_attack(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }
    var slot = candidates[cand_idx];
    let len = slot.len;

    // Byte-swap each input word: host packs LE; SHA-1 wants BE message words.
    var M: array<u32, 16>;
    for (var i = 0u; i < 14u; i = i + 1u) {
        M[i] = byteswap(slot.bytes[i]);
    }
    M = pad_be_block(M, len);
    let h = sha1_block(M);
    scan_targets_sha1(h, cand_idx, params.num_targets, params.max_matches);
}
