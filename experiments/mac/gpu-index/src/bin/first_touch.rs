//! CPU first-touch controls for anonymous VM and Metal shared allocations.
//! This measures CPU faults, not GPU faults, and does not dispatch GPU work.
use metal::{Device, MTLResourceOptions};
use serde_json::{Value, json};
use std::{error::Error, fs, mem::MaybeUninit, path::PathBuf, ptr, time::Instant};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

#[derive(Clone, Copy)]
struct Faults {
    minor: i64,
    major: i64,
}
impl Faults {
    fn now() -> Result<Self> {
        let mut usage = MaybeUninit::<libc::rusage>::uninit();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let usage = unsafe { usage.assume_init() };
        Ok(Self {
            minor: usage.ru_minflt,
            major: usage.ru_majflt,
        })
    }
    fn delta(self, before: Self) -> Value {
        json!({"minor":self.minor-before.minor,"major":self.major-before.major})
    }
}

fn phase<T>(work: impl FnOnce() -> Result<T>) -> Result<(T, Value)> {
    let before = Faults::now()?;
    let start = Instant::now();
    let value = work()?;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    let after = Faults::now()?;
    Ok((
        value,
        json!({"wall_ms":elapsed,"cpu_process_faults":after.delta(before)}),
    ))
}

enum Memory {
    Anonymous { ptr: *mut libc::c_void, len: usize },
    Shared(metal::Buffer),
}
impl Memory {
    fn new(device: &metal::DeviceRef, bytes: usize, shared: bool) -> Result<Self> {
        if shared {
            if bytes as u64 > device.max_buffer_length() {
                return Err("Metal buffer too large".into());
            }
            let buffer = device.new_buffer(
                bytes as u64,
                MTLResourceOptions::StorageModeShared
                    | MTLResourceOptions::HazardTrackingModeTracked,
            );
            if buffer.contents().is_null() {
                return Err("shared buffer has no CPU mapping".into());
            }
            Ok(Self::Shared(buffer))
        } else {
            let ptr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    bytes,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self::Anonymous { ptr, len: bytes })
        }
    }
    fn pointer(&self) -> *mut u8 {
        match self {
            Self::Anonymous { ptr, .. } => ptr.cast(),
            Self::Shared(buffer) => buffer.contents().cast(),
        }
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        if let Self::Anonymous { ptr, len } = self {
            assert_eq!(unsafe { libc::munmap(*ptr, *len) }, 0, "munmap failed");
        }
    }
}

fn sample(device: &metal::DeviceRef, source: &[u8], shared: bool, copy: bool) -> Result<Value> {
    let bytes = source.len();
    let (memory, allocation) = phase(|| Memory::new(device, bytes, shared))?;
    let write = || -> Result<()> {
        unsafe {
            if copy {
                libc::memcpy(memory.pointer().cast(), source.as_ptr().cast(), bytes);
            } else {
                libc::memset(memory.pointer().cast(), 0xa5, bytes);
            }
        }
        Ok(())
    };
    let (_, first_write) = phase(write)?;
    // Reading before initialization would not be a valid Metal-memory control.
    // Each entire region is checked after writing and before it is overwritten.
    let (_, first_read) = phase(|| {
        assert!(
            unsafe { std::slice::from_raw_parts(memory.pointer(), bytes) }
                .iter()
                .all(|&value| value == 0xa5)
        );
        Ok(())
    })?;
    let (_, second_write) = phase(write)?;
    let (_, second_read) = phase(|| {
        assert!(
            unsafe { std::slice::from_raw_parts(memory.pointer(), bytes) }
                .iter()
                .all(|&value| value == 0xa5)
        );
        Ok(())
    })?;
    let (_, release) = phase(|| {
        drop(memory);
        Ok(())
    })?;
    Ok(
        json!({"allocation":allocation,"first_write":first_write,"first_read":first_read,"second_write":second_write,"second_read":second_read,"release":release}),
    )
}

fn stats(samples: &[Value], phase: &str, field: &str) -> Value {
    let mut values = samples
        .iter()
        .map(|s| {
            if field == "wall_ms" {
                s[phase][field].as_f64().unwrap()
            } else {
                s[phase]["cpu_process_faults"][field].as_f64().unwrap()
            }
        })
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    json!({"min":values[0],"median":values[values.len()/2],"max":values[values.len()-1]})
}

fn main() -> Result<()> {
    let mut sizes = vec![8usize << 20, 128 << 20, 512 << 20];
    let mut runs = 9usize;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        match flag.as_str() {
            "--bytes" => {
                sizes = value
                    .split(',')
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?
            }
            "--runs" => runs = value.parse()?,
            "--out" => out = Some(value.into()),
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    if runs < 3 || runs % 2 == 0 || sizes.is_empty() || sizes.contains(&0) {
        return Err("odd runs >=3 and positive byte sizes required".into());
    }
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err("cannot determine host page size".into());
    }
    let device = Device::system_default().ok_or("Metal device required")?;
    let _ = Faults::now()?;
    let mut results = Vec::new();
    println!(
        "bytes\tallocation\twrite\tallocation_ms\tfirst_write_ms\tsecond_write_ms\tfirst_write_minor\tsecond_write_minor"
    );
    for bytes in sizes {
        let source = vec![0xa5u8; bytes];
        let modes = [(false, false), (false, true), (true, false), (true, true)];
        let mut samples: Vec<Vec<Value>> =
            (0..modes.len()).map(|_| Vec::with_capacity(runs)).collect();
        for round in 0..runs {
            for step in 0..modes.len() {
                let slot = (round + step) % modes.len();
                let (shared, copy) = modes[slot];
                let result =
                    metal::objc::rc::autoreleasepool(|| sample(&device, &source, shared, copy))?;
                samples[slot].push(result);
            }
        }
        for ((shared, copy), samples) in modes.into_iter().zip(samples) {
            let allocation = if shared {
                "metal_shared"
            } else {
                "anonymous_mmap"
            };
            let operation = if copy { "memcpy" } else { "memset" };
            let mut summary = serde_json::Map::new();
            for phase in [
                "allocation",
                "first_write",
                "first_read",
                "second_write",
                "second_read",
                "release",
            ] {
                summary.insert(phase.into(), json!({"wall_ms":stats(&samples,phase,"wall_ms"),"minor":stats(&samples,phase,"minor"),"major":stats(&samples,phase,"major")}));
            }
            println!(
                "{bytes}\t{allocation}\t{operation}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}",
                summary["allocation"]["wall_ms"]["median"].as_f64().unwrap(),
                summary["first_write"]["wall_ms"]["median"]
                    .as_f64()
                    .unwrap(),
                summary["second_write"]["wall_ms"]["median"]
                    .as_f64()
                    .unwrap(),
                summary["first_write"]["minor"]["median"],
                summary["second_write"]["minor"]["median"]
            );
            results.push(json!({"bytes":bytes,"allocation":allocation,"operation":operation,"summary":summary,"samples":samples}));
        }
    }
    let report = json!({"device":device.name(),"page_bytes":page_size,"runs":runs,
        "method":"fresh virtual allocation each sample; resident source initialized outside timing; same byte count/pattern in first and second writes; full reads between writes validate initialization",
        "limits":"CPU process getrusage minor/major fault deltas, not GPU page faults and not a zero-fill-only counter. No GPU commands. Source/cache warming and first-touch costs coexist; fresh allocations do not guarantee cold physical pages.","results":results});
    if let Some(path) = out {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        serde_json::to_writer_pretty(fs::File::create(&path)?, &report)?;
        println!("Raw first-touch evidence: {}", path.display());
    }
    Ok(())
}
