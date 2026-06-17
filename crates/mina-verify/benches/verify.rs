//! Benchmark the indexer hot path: decode + verify one precomputed block's SNARK proof.
//!
//! A single verification is seconds long, so the value here is a real wall-clock number
//! (ms/block, blocks/sec) — not micro-benchmark statistics. Hence a plain `harness = false`
//! binary instead of criterion: no extra dependencies, and `cargo bench` runs it in
//! release. Verifier construction (the one-time VK load) is timed separately from the
//! per-block verify so you can see both costs.
//!
//! ```text
//! cargo bench -p mina-verify
//! BENCH_ITERS=10 cargo bench -p mina-verify   # more samples
//! ```

use std::time::Instant;

use mina_verify::Verifier;

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/devnet-528700.json"
);

fn main() {
    let iters: usize = std::env::var("BENCH_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(5);

    let bytes = std::fs::read(FIXTURE_PATH).expect("read fixture block");
    let body = String::from_utf8_lossy(&bytes).into_owned();

    let t0 = Instant::now();
    let verifier = Verifier::for_network_offline("devnet").expect("embedded devnet VK");
    let build = t0.elapsed();
    println!("verifier build (one-time VK load): {:.3?}", build);

    // Warm up (first verify also primes the globally-cached SRS).
    let warm = Instant::now();
    let vb = verifier
        .verify_precomputed_and_extract(&body)
        .expect("fixture must verify");
    println!(
        "warmup verify: {:.3?}  (height {}, state {})",
        warm.elapsed(),
        vb.height,
        vb.state_hash
    );

    let mut samples = Vec::with_capacity(iters);
    for i in 0..iters {
        let t = Instant::now();
        let r = verifier.verify_precomputed_and_extract(&body);
        let dt = t.elapsed();
        assert!(r.is_ok(), "verification failed on iter {i}");
        samples.push(dt);
        println!("  iter {i}: {:.3?}", dt);
    }

    samples.sort();
    let n = samples.len() as u32;
    let total: std::time::Duration = samples.iter().sum();
    let mean = total / n;
    let median = samples[samples.len() / 2];
    let min = samples[0];
    let max = samples[samples.len() - 1];
    let per_sec = 1.0 / mean.as_secs_f64();

    println!("\n=== decode + verify, {iters} iters ===");
    println!("  min    {:.3?}", min);
    println!("  median {:.3?}", median);
    println!("  mean   {:.3?}", mean);
    println!("  max    {:.3?}", max);
    println!("  throughput ~{per_sec:.2} blocks/sec/core");
}
