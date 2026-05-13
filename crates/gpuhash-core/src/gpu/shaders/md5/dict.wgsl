// MD5 dictionary entry point. Depends on:
//   match_common.wgsl   — targets, match_buf, rotl
//   dict_common.wgsl    — CandidateSlot, Params, candidates, params
//   md5_funcs.wgsl      — K_MD5, S_MD5, md5_block, md5_pad_block, scan_targets_md5

@compute @workgroup_size(64)
fn md5_attack(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }
    // Copy into a var so the inner `slot.bytes[i]` access can use a dynamic
    // index (naga forbids it on let-bound array values).
    var slot = candidates[cand_idx];
    let len = slot.len;

    var M: array<u32, 16>;
    for (var i = 0u; i < 14u; i = i + 1u) {
        M[i] = slot.bytes[i];
    }
    M = md5_pad_block(M, len);
    let h = md5_block(M);
    scan_targets_md5(h, cand_idx, params.num_targets, params.max_matches);
}
