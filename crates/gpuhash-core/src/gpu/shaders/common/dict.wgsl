// Inputs and parameters shared by every algorithm's dictionary-mode entry
// point. The host packs candidate bytes little-endian into 14 u32 words per
// `CandidateSlot`; algorithms that operate on big-endian message words (SHA-1,
// SHA-256) byte-swap on read.

struct CandidateSlot {
    len:   u32,
    bytes: array<u32, 14>,
}

struct Params {
    num_candidates: u32,
    num_targets:    u32,
    max_matches:    u32,
    _pad:           u32,
}

@group(0) @binding(0) var<storage, read> candidates: array<CandidateSlot>;
@group(0) @binding(3) var<uniform>       params:     Params;
