use criterion::{black_box, criterion_group, criterion_main, Criterion};

use std::convert::TryInto;

use crucible::ShuffleContext;

use rand::{distributions::Alphanumeric, Rng};

pub fn rng_gen_range_100gb_benchmark(c: &mut Criterion) {
    let num_blocks: usize = 100 * 1024 * 1024 * 1024 / 4096;
    c.bench_function("rng::gen_range for 100 GB volume, 4k sectors", |b| {
        let mut rng = rand::thread_rng();
        b.iter(|| rng.gen_range(0..num_blocks))
    });
}

pub fn shuffle_100gb_benchmark(c: &mut Criterion) {
    let random_key: [u8; 32] = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .collect::<Vec<u8>>()
        .try_into()
        .expect("expected vector of 32!");

    let num_blocks: usize = 100 * 1024 * 1024 * 1024 / 4096;
    let mut sc = ShuffleContext::new(&random_key, num_blocks);

    c.bench_function("shuffle context for 100 GB volume, 4k sectors", |b| {
        let mut rng = rand::thread_rng();

        b.iter(|| {
            let index = rng.gen_range(0..num_blocks);
            sc.index(index as u64)
        })
    });
}

pub fn shuffle_2tb_benchmark(c: &mut Criterion) {
    let random_key: [u8; 32] = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .collect::<Vec<u8>>()
        .try_into()
        .expect("expected vector of 32!");

    let num_blocks: usize = 2 * 1024 * 1024 * 1024 * 1024 / 4096;
    let mut sc = ShuffleContext::new(&random_key, num_blocks);

    c.bench_function("shuffle context for 2 TB volume, 4k sectors", |b| {
        let mut rng = rand::thread_rng();

        b.iter(|| {
            let index = rng.gen_range(0..num_blocks);
            sc.index(index as u64)
        })
    });
}

criterion_group!(
    benches,
    rng_gen_range_100gb_benchmark,
    shuffle_100gb_benchmark,
    shuffle_2tb_benchmark
);

criterion_main!(benches);
