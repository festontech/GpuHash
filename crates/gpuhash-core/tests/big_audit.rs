//! Integration test for a "big" end-to-end dictionary audit.
//!
//! The earlier unit tests in `gpuhash-core` exercise individual modules (digest
//! correctness, GPU↔CPU agreement on small inputs). This test stresses the
//! *whole pipeline*: a synthetic 10 000-candidate wordlist with 50 randomly
//! planted hashes, driven through `Engine::run` exactly the way the CLI and
//! Tauri shells do it.
//!
//! Deterministic by design — the PRNG is seeded so the test is reproducible
//! across runs and platforms. Costs ~50 ms in release mode on the CPU path;
//! the GPU path is skipped here to keep `cargo test` headless-CI-friendly.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use gpuhash_core::digest::{digest, to_hex};
use gpuhash_core::{Algorithm, AttackConfig, AttackMode, Backend, Engine, EngineEvent, GpuTuning};

const CANDIDATE_COUNT: usize = 10_000;
const PLANT_COUNT: usize = 50;
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Minimal xorshift64* PRNG — deterministic, no dependency cost.
fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn random_password(rng: &mut u64) -> String {
    // Lowercase ASCII, 4–8 chars. Fits comfortably inside MAX_CANDIDATE_LEN
    // and matches the kind of charset hashcat-style audits actually see.
    let len = 4 + (xorshift(rng) % 5) as usize;
    let mut s = String::with_capacity(len);
    for _ in 0..len {
        let n = (xorshift(rng) % 26) as u8;
        s.push((b'a' + n) as char);
    }
    s
}

struct Corpus {
    wordlist_path: PathBuf,
    hashes_path: PathBuf,
    planted: HashSet<String>,
    tmpdir: PathBuf,
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmpdir);
    }
}

fn build_corpus(algo: Algorithm) -> Corpus {
    let tmpdir = std::env::temp_dir().join(format!(
        "gpuhash-big-audit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&tmpdir).expect("create tmpdir");

    let mut rng = SEED;
    let mut candidates: Vec<String> = (0..CANDIDATE_COUNT)
        .map(|_| random_password(&mut rng))
        .collect();
    // Deduplicate while preserving order — collisions on 4–8 char lowercase
    // happen but are rare; we just want a stable count for assertions.
    let mut seen: HashSet<String> = HashSet::new();
    candidates.retain(|c| seen.insert(c.clone()));

    // Pick `PLANT_COUNT` distinct random positions to plant as targets.
    let mut planted: HashSet<String> = HashSet::new();
    while planted.len() < PLANT_COUNT {
        let idx = (xorshift(&mut rng) as usize) % candidates.len();
        planted.insert(candidates[idx].clone());
    }

    let wordlist_path = tmpdir.join("wordlist.txt");
    {
        let mut f = fs::File::create(&wordlist_path).expect("create wordlist");
        for c in &candidates {
            writeln!(f, "{c}").unwrap();
        }
    }

    let hashes_path = tmpdir.join("hashes.txt");
    {
        let mut f = fs::File::create(&hashes_path).expect("create hashes");
        for plain in &planted {
            let d = digest(algo, plain.as_bytes()).expect("digest");
            writeln!(f, "{}", to_hex(&d)).unwrap();
        }
    }

    Corpus {
        wordlist_path,
        hashes_path,
        planted,
        tmpdir,
    }
}

async fn run_audit(corpus: &Corpus, algo: Algorithm) -> HashSet<String> {
    let cfg = AttackConfig {
        algo,
        hashes_path: corpus.hashes_path.clone(),
        mode: AttackMode::Dictionary {
            wordlist: corpus.wordlist_path.clone(),
        },
        backend: Backend::Cpu,
        gpu_tuning: GpuTuning::default(),
        session_name: None,
    };

    let mut running = Engine::new().run(cfg);
    let mut matches = HashSet::new();
    let mut finished = false;
    while let Some(ev) = running.next_event().await {
        match ev {
            EngineEvent::Match { plaintext, .. } => {
                matches.insert(plaintext);
            }
            EngineEvent::Finished { .. } => finished = true,
            EngineEvent::Error { message } => panic!("engine error: {message}"),
            _ => {}
        }
    }
    assert!(finished, "engine did not emit Finished");
    matches
}

#[tokio::test(flavor = "multi_thread")]
async fn big_dictionary_audit_finds_all_planted_md5() {
    let corpus = build_corpus(Algorithm::Md5);
    let found = run_audit(&corpus, Algorithm::Md5).await;
    assert_eq!(
        found, corpus.planted,
        "found set must equal planted set (no missing, no extras)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn big_dictionary_audit_finds_all_planted_sha256() {
    let corpus = build_corpus(Algorithm::Sha256);
    let found = run_audit(&corpus, Algorithm::Sha256).await;
    assert_eq!(found, corpus.planted);
}
