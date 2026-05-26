//! Generate the synthetic example corpora committed under `examples/`.
//!
//! Builds three demo scenarios so anyone with a fresh checkout can drive
//! the CLI or the Tauri shell against non-trivial workloads:
//!
//! 1. `examples/medium_dict.txt`       — 1 000 unique 4-8 char lowercase words
//!    `examples/medium_md5.txt`        — 50 planted MD5 digests
//!    `examples/medium_sha1.txt`       — 50 planted SHA-1 digests
//!    `examples/medium_sha256.txt`     — 50 planted SHA-256 digests
//!
//! 2. `examples/big_dict.txt`          — 50 000 unique 4-8 char candidates
//!    `examples/big_md5.txt`           — 200 planted MD5 digests
//!
//! 3. `examples/brute_4lower_md5.txt`  — 50 MD5 digests of random 4-char
//!    lowercase plaintexts; pair with `--mask ?l?l?l?l`
//! 4. `examples/brute_5lower_md5.txt`  — 50 MD5 digests of random 5-char
//!    lowercase plaintexts; pair with `--mask ?l?l?l?l?l` (26⁵ ≈ 11.9 M
//!    keyspace — runs the GPU pipeline long enough to show steady-state
//!    H/s in the Live chart)
//!
//! Deterministic: re-running with the default seed produces byte-identical
//! files. Pass a different seed via `cargo run --example gen_examples -- <seed>`.
//!
//! Plaintexts are random ASCII strings — collisions with real-world
//! credentials are accidental and the corpora carry no leak provenance.
//!
//! Run from the repo root:
//!
//! ```pwsh
//! cargo run --release --example gen_examples -p gpuhash-core
//! ```

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use gpuhash_core::digest::{digest, to_hex};
use gpuhash_core::Algorithm;

const DEFAULT_SEED: u64 = 42;
const MEDIUM_COUNT: usize = 1_000;
const MEDIUM_PLANTED: usize = 50;
const BIG_COUNT: usize = 50_000;
const BIG_PLANTED: usize = 200;
const BRUTE_PLANTED: usize = 50;

fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn random_word(rng: &mut u64, min_len: usize, max_len: usize) -> String {
    let len = if min_len == max_len {
        min_len
    } else {
        min_len + (xorshift(rng) as usize) % (max_len - min_len + 1)
    };
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        let n = (xorshift(rng) % 26) as u8;
        s.push((b'a' + n) as char);
    }
    s
}

/// Build `count` unique random words in insertion order.
fn unique_corpus(rng: &mut u64, count: usize, min_len: usize, max_len: usize) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::with_capacity(count);
    let mut out: Vec<String> = Vec::with_capacity(count);
    while out.len() < count {
        let w = random_word(rng, min_len, max_len);
        if seen.insert(w.clone()) {
            out.push(w);
        }
    }
    out
}

/// Pick `count` distinct indices from `[0, pool_size)`, sorted ascending.
fn pick_planted_indices(rng: &mut u64, count: usize, pool_size: usize) -> Vec<usize> {
    assert!(count <= pool_size, "planted={count} > pool={pool_size}");
    let mut seen: HashSet<usize> = HashSet::with_capacity(count);
    while seen.len() < count {
        let idx = (xorshift(rng) as usize) % pool_size;
        seen.insert(idx);
    }
    let mut v: Vec<usize> = seen.into_iter().collect();
    v.sort_unstable();
    v
}

fn write_lines(path: &PathBuf, lines: &[String]) {
    let mut f = fs::File::create(path).unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    for line in lines {
        // LF only so diffs stay portable; trailing newline so POSIX tools
        // see a complete final record.
        writeln!(f, "{line}").unwrap();
    }
}

fn hashes_for(plaintexts: &[String], algo: Algorithm) -> Vec<String> {
    plaintexts
        .iter()
        .map(|p| to_hex(&digest(algo, p.as_bytes()).expect("digest")))
        .collect()
}

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SEED);

    let examples = repo_examples_dir();
    println!("Writing into {}", examples.display());

    // Scenario 1 — medium dict + all three algorithms.
    println!(
        "Scenario 1: medium dict ({MEDIUM_COUNT} candidates, {MEDIUM_PLANTED} planted, md5/sha1/sha256)..."
    );
    let mut rng1 = seed.max(1);
    let medium = unique_corpus(&mut rng1, MEDIUM_COUNT, 4, 8);
    let medium_idx = pick_planted_indices(&mut rng1, MEDIUM_PLANTED, medium.len());
    let medium_planted: Vec<String> = medium_idx.iter().map(|&i| medium[i].clone()).collect();

    write_lines(&examples.join("medium_dict.txt"), &medium);
    for algo in [Algorithm::Md5, Algorithm::Sha1, Algorithm::Sha256] {
        let hashes = hashes_for(&medium_planted, algo);
        write_lines(
            &examples.join(format!("medium_{}.txt", algo.name())),
            &hashes,
        );
    }

    // Scenario 2 — big dict + MD5 hashes.
    println!("Scenario 2: big dict ({BIG_COUNT} candidates, {BIG_PLANTED} planted MD5)...");
    let mut rng2 = seed.wrapping_add(1).max(1);
    let big = unique_corpus(&mut rng2, BIG_COUNT, 4, 8);
    let big_idx = pick_planted_indices(&mut rng2, BIG_PLANTED, big.len());
    let big_planted: Vec<String> = big_idx.iter().map(|&i| big[i].clone()).collect();

    write_lines(&examples.join("big_dict.txt"), &big);
    write_lines(
        &examples.join("big_md5.txt"),
        &hashes_for(&big_planted, Algorithm::Md5),
    );

    // Scenario 3 — bruteforce-friendly 4-lowercase MD5 hashes.
    println!("Scenario 3: brute_4lower_md5 ({BRUTE_PLANTED} hashes)...");
    let mut rng3 = seed.wrapping_add(2).max(1);
    let brute4 = unique_corpus(&mut rng3, BRUTE_PLANTED, 4, 4);
    write_lines(
        &examples.join("brute_4lower_md5.txt"),
        &hashes_for(&brute4, Algorithm::Md5),
    );

    // Scenario 4 — 5-lowercase mask. ~11.9 M keyspace, enough to give the
    // GPU pipeline a couple seconds of steady-state work for the chart demo.
    println!("Scenario 4: brute_5lower_md5 ({BRUTE_PLANTED} hashes)...");
    let mut rng4 = seed.wrapping_add(3).max(1);
    let brute5 = unique_corpus(&mut rng4, BRUTE_PLANTED, 5, 5);
    write_lines(
        &examples.join("brute_5lower_md5.txt"),
        &hashes_for(&brute5, Algorithm::Md5),
    );

    println!("\nDone. Generated:");
    for name in [
        "medium_dict.txt",
        "medium_md5.txt",
        "medium_sha1.txt",
        "medium_sha256.txt",
        "big_dict.txt",
        "big_md5.txt",
        "brute_4lower_md5.txt",
        "brute_5lower_md5.txt",
    ] {
        let p = examples.join(name);
        let size = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        println!("  {name:<30}  {size:>10} bytes");
    }
}

/// `examples/` directory at the workspace root. `CARGO_MANIFEST_DIR` points
/// at `crates/gpuhash-core/`, so we walk two levels up.
fn repo_examples_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("examples"))
        .expect("workspace layout");
    assert!(
        dir.exists(),
        "examples/ not found at {} — run from the workspace root",
        dir.display()
    );
    dir
}
