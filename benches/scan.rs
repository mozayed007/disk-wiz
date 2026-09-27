//! Scanner benchmarks on a synthetic tree.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use disk_wiz::scan::{ScanOptions, scan};
use std::fs;
use std::path::Path;

fn build_tree(root: &Path, dirs: usize, files_per_dir: usize) {
    for d in 0..dirs {
        let dir = root.join(format!("dir{d:04}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..files_per_dir {
            let data = vec![b'x'; 1024 + (f % 8) * 256];
            fs::write(dir.join(format!("file{f:03}.bin")), data).unwrap();
        }
        if d % 16 == 0 {
            let nested = dir.join("nested");
            fs::create_dir_all(&nested).unwrap();
            fs::write(nested.join("deep.bin"), vec![b'y'; 4096]).unwrap();
        }
    }
}

fn bench_scan(c: &mut Criterion) {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = 200;
    let files_per_dir = 25;
    build_tree(tmp.path(), dirs, files_per_dir);
    let total_files = dirs * (files_per_dir + 1);

    let mut group = c.benchmark_group("scan");
    group.throughput(Throughput::Elements(total_files as u64));
    let opts = ScanOptions::default();
    group.bench_function("synthetic_tree", |b| {
        b.iter(|| {
            let outcome = scan(tmp.path(), &opts);
            assert!(outcome.root.size > 0);
        });
    });
    group.finish();
}

fn bench_layout(c: &mut Criterion) {
    let mut weights: Vec<u64> = (0..10_000u64).map(|i| 1_000_000 / (i + 1) + 1).collect();
    weights.sort_unstable_by(|a, b| b.cmp(a));
    let area = ratatui::layout::Rect::new(0, 0, 200, 60);

    let mut group = c.benchmark_group("layout");
    group.throughput(Throughput::Elements(weights.len() as u64));
    group.bench_function("squarify_10k", |b| {
        b.iter(|| {
            let (placed, _dropped) = disk_wiz::layout::treemap(&weights, area);
            assert!(!placed.is_empty());
        });
    });
    group.finish();
}

criterion_group!(benches, bench_scan, bench_layout);
criterion_main!(benches);
