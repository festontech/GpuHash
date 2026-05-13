// MD5 bruteforce entry point. Depends on:
//   match_common.wgsl        — targets, match_buf, rotl
//   bruteforce_common.wgsl   — MaskPos, Params, mask, params, synthesize_candidate_le
//   md5_funcs.wgsl           — md5_block, md5_pad_block, scan_targets_md5

@compute @workgroup_size(64)
fn md5_bruteforce(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }
    let abs_idx = params.base_index + cand_idx;
    let len = params.num_positions;

    // Candidate bytes packed LE into 16 u32 words — MD5's natural form.
    var M = synthesize_candidate_le(abs_idx, len);
    M = md5_pad_block(M, len);
    let h = md5_block(M);
    scan_targets_md5(h, cand_idx, params.num_targets, params.max_matches);
}
