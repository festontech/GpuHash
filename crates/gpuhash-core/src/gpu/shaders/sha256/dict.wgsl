// SHA-256 dictionary entry point. Depends on:
//   common/match.wgsl    — targets, match_buf, rotl, rotr, byteswap, pad_be_block
//   common/dict.wgsl     — CandidateSlot, Params, candidates, params
//   sha256/funcs.wgsl    — sha256_block, scan_targets_sha256

@compute @workgroup_size(64)
fn sha256_attack(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }
    var slot = candidates[cand_idx];
    let len = slot.len;

    // Host packs LE; SHA-256 wants BE message words.
    var M: array<u32, 16>;
    for (var i = 0u; i < 14u; i = i + 1u) {
        M[i] = byteswap(slot.bytes[i]);
    }
    M = pad_be_block(M, len);
    let h = sha256_block(M);
    scan_targets_sha256(h, cand_idx, params.num_targets, params.max_matches);
}
