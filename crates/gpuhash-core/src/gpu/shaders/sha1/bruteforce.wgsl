// SHA-1 bruteforce entry point. Depends on:
//   match_common.wgsl       — targets, match_buf, rotl, byteswap
//   bruteforce_common.wgsl  — MaskPos, Params, mask, params, synthesize_candidate_le
//   sha1_funcs.wgsl         — sha1_block, sha1_pad_block, scan_targets_sha1

@compute @workgroup_size(64)
fn sha1_bruteforce(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }
    let abs_idx = params.base_index + cand_idx;
    let len = params.num_positions;

    // synthesize_candidate_le returns LE-packed bytes; SHA-1 wants BE words.
    // Copy into a var so the inner `bytes_le[i]` access is allowed to use a
    // dynamic index (naga rejects it on let-bound array values).
    var bytes_le = synthesize_candidate_le(abs_idx, len);
    var M: array<u32, 16>;
    for (var i = 0u; i < 14u; i = i + 1u) {
        M[i] = byteswap(bytes_le[i]);
    }
    M = pad_be_block(M, len);
    let h = sha1_block(M);
    scan_targets_sha1(h, cand_idx, params.num_targets, params.max_matches);
}
