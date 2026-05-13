// SHA-256 bruteforce entry point. Depends on:
//   common/match.wgsl        — targets, match_buf, rotl, rotr, byteswap, pad_be_block
//   common/bruteforce.wgsl   — MaskPos, Params, mask, params, synthesize_candidate_le
//   sha256/funcs.wgsl        — sha256_block, scan_targets_sha256

@compute @workgroup_size(64)
fn sha256_bruteforce(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cand_idx = gid.x;
    if (cand_idx >= params.num_candidates) {
        return;
    }
    let abs_idx = params.base_index + cand_idx;
    let len = params.num_positions;

    // synthesize_candidate_le returns LE-packed bytes; SHA-256 wants BE words.
    // Copy into a var so the inner index is allowed to be dynamic.
    var bytes_le = synthesize_candidate_le(abs_idx, len);
    var M: array<u32, 16>;
    for (var i = 0u; i < 14u; i = i + 1u) {
        M[i] = byteswap(bytes_le[i]);
    }
    M = pad_be_block(M, len);
    let h = sha256_block(M);
    scan_targets_sha256(h, cand_idx, params.num_targets, params.max_matches);
}
