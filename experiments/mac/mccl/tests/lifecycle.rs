use mccl::{memory, Gpu, Result};

struct RestoreLimits {
    cache: Option<usize>,
    wired: Option<usize>,
}

impl RestoreLimits {
    fn restore(&mut self) -> Result<()> {
        if let Some(previous) = self.wired {
            memory::set_wired_limit(previous)?;
            self.wired = None;
        }
        if let Some(previous) = self.cache {
            memory::set_cache_limit(previous)?;
            self.cache = None;
        }
        Ok(())
    }
}

impl Drop for RestoreLimits {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            if std::thread::panicking() {
                eprintln!("could not restore MLX memory limits during unwinding: {error}");
            } else {
                panic!("could not restore MLX memory limits: {error}");
            }
        }
    }
}

/// Keep all device work and process-wide limit changes in one test/thread.
/// Requires the built native libraries and a Metal GPU, like gpu_check.rs.
#[test]
fn shared_storage_and_lazy_graph_lifetimes() -> Result<()> {
    // Memory controls must also work before any context installs the error handler.
    memory::stats()?;
    memory::clear_cache()?;

    let gpu = Gpu::new()?;
    let mut source = vec![0, 1, 1, 7, u32::MAX];
    let original = source.clone();
    let uploaded = gpu.upload(&source)?;
    source.fill(42);
    drop(source);
    assert_eq!(uploaded.as_slice()?, original.as_slice());

    let other = gpu.upload(&[0, 2, u32::MAX])?;
    let merged = gpu.merge(&uploaded, &other)?;
    let output = gpu.copy(&merged)?;
    drop(uploaded);
    drop(other);
    drop(merged);
    let expected = [0, 0, 1, 1, 2, 7, u32::MAX, u32::MAX];
    let borrowed = output.as_slice()?;
    assert_eq!(borrowed, expected);

    // Install the rollback guard before changing the second global budget.
    let mut restore = RestoreLimits {
        cache: Some(memory::set_cache_limit(0)?),
        wired: None,
    };
    restore.wired = Some(memory::set_wired_limit(0)?);
    memory::clear_cache()?;
    assert_eq!(borrowed, expected);

    // Exercise both uncached and pooled allocations without ever releasing the
    // original output or its borrowed slice. No pointer/counter assumptions.
    for cache_limit in [0, 8 * 1024 * 1024] {
        memory::set_cache_limit(cache_limit)?;
        for round in 0..16_u32 {
            let values: Vec<u32> = (0..(round as usize * 257 + 1))
                .map(|i| (i / 3) as u32 + round)
                .collect();
            let fresh = gpu.upload(&values)?;
            let copied = gpu.copy(&fresh)?;
            let combined = gpu.merge(&output, &copied)?;
            let mut combined_expected = expected.to_vec();
            combined_expected.extend_from_slice(&values);
            combined_expected.sort_unstable();
            drop(fresh);
            assert_eq!(copied.as_slice()?, values.as_slice());
            drop(copied);
            assert_eq!(combined.as_slice()?, combined_expected.as_slice());
            drop(combined);
            if round % 3 == 0 {
                memory::clear_cache()?;
            }
            assert_eq!(borrowed, expected);
        }
    }
    gpu.synchronize()?;
    memory::stats()?;
    memory::clear_cache()?;
    assert_eq!(borrowed, expected);
    assert_eq!(output.to_vec()?, expected);
    restore.restore()?;

    // The only Rust owner escaping this scope is an unevaluated final result.
    // MLX must retain every dependency and the wrapper must retain its context.
    let pending = {
        let temporary_gpu = Gpu::new()?;
        let left = temporary_gpu.upload(&[0, 4, 4, u32::MAX])?;
        let right = temporary_gpu.upload(&[1, 4, 9])?;
        let merge = temporary_gpu.merge(&left, &right)?;
        temporary_gpu.copy(&merge)?
    };
    memory::clear_cache()?;
    for value in 0..8 {
        let scratch = gpu.upload(&vec![value; 4096])?;
        gpu.copy(&scratch)?.eval()?;
    }
    assert_eq!(pending.as_slice()?, [0, 1, 4, 4, 4, 9, u32::MAX]);
    memory::clear_cache()?;
    assert_eq!(pending.to_vec()?, [0, 1, 4, 4, 4, 9, u32::MAX]);

    // Cover empty storage, extrema, duplicate keys, and strongly unequal inputs.
    let cases: Vec<(Vec<u32>, Vec<u32>)> = vec![
        (vec![], vec![]),
        (vec![], vec![0, u32::MAX]),
        (vec![0, u32::MAX], vec![]),
        (vec![u32::MAX], vec![0, 0, 1, 1, 2, 3, u32::MAX]),
        (vec![0, 1, 1, 2, 3, u32::MAX], vec![0]),
        (vec![0, 0, u32::MAX], vec![0, u32::MAX, u32::MAX]),
    ];
    for (left, right) in cases {
        let a = gpu.upload(&left)?;
        let b = gpu.upload(&right)?;
        let result = gpu.merge(&a, &b)?;
        let copy = gpu.copy(&result)?;
        let mut expected = left;
        expected.extend_from_slice(&right);
        expected.sort_unstable();
        drop(a);
        drop(b);
        assert_eq!(result.len(), expected.len());
        assert_eq!(result.is_empty(), expected.is_empty());
        assert_eq!(result.as_slice()?, expected.as_slice());
        drop(result);
        assert_eq!(copy.to_vec()?, expected);
    }

    let unrelated = Gpu::new()?;
    let unrelated_input = unrelated.upload(&[0])?;
    assert!(unrelated.copy(&output).is_err());
    assert!(gpu.merge(&output, &unrelated_input).is_err());
    assert_eq!(borrowed, [0, 0, 1, 1, 2, 7, u32::MAX, u32::MAX]);
    gpu.synchronize()?;
    Ok(())
}
