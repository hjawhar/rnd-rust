//! Benchmarks for inference and storage operations.

use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_placeholder(c: &mut Criterion) {
    c.bench_function("noop", |b| b.iter(|| 1 + 1));
}

criterion_group!(benches, benchmark_placeholder);
criterion_main!(benches);
