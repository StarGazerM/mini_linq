use gpu_triangles::Error;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    time::Instant,
};

pub struct Edges {
    pub sources: Vec<i64>,
    pub targets: Vec<i64>,
    pub input_rows: usize,
    pub preparation_ms: f64,
}
impl Edges {
    pub fn load(path: &str) -> Result<Self, Error> {
        let start = Instant::now();
        let mut rows = Vec::new();
        let mut line = String::new();
        let mut reader = BufReader::with_capacity(1 << 20, File::open(path)?);
        while reader.read_line(&mut line)? != 0 {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                let mut fields = trimmed.split_whitespace();
                let a = fields.next().ok_or("missing source")?.parse()?;
                let b = fields.next().ok_or("missing target")?.parse()?;
                if fields.next().is_some() {
                    return Err("edge input has more than two columns".into());
                }
                rows.push((a, b));
            }
            line.clear();
        }
        let mut edges = Self::canonical(rows);
        edges.preparation_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(edges)
    }
    pub fn canonical(mut rows: Vec<(i64, i64)>) -> Self {
        let input_rows = rows.len();
        rows.sort_unstable();
        rows.dedup();
        let (sources, targets) = rows.into_iter().unzip();
        Self {
            sources,
            targets,
            input_rows,
            preparation_ms: 0.0,
        }
    }
    pub fn domain(&self) -> (i64, i64) {
        self.sources
            .first()
            .zip(self.sources.last())
            .map(|(&a, &b)| (a, b))
            .unwrap_or((0, -1))
    }
    pub fn range(&self, key: i64) -> std::ops::Range<usize> {
        self.sources.partition_point(|&x| x < key)..self.sources.partition_point(|&x| x <= key)
    }
    pub fn wedges(&self) -> u64 {
        self.targets
            .iter()
            .map(|&b| self.range(b).len() as u64)
            .sum()
    }
    pub fn cpu_count(&self) -> u64 {
        self.sources
            .iter()
            .zip(&self.targets)
            .map(|(&a, &b)| {
                let ar = self.range(a);
                let br = self.range(b);
                let (small, large) = if ar.len() < br.len() {
                    (ar, br)
                } else {
                    (br, ar)
                };
                self.targets[small]
                    .iter()
                    .filter(|&&c| self.targets[large.clone()].binary_search(&c).is_ok())
                    .count() as u64
            })
            .sum()
    }
    pub fn pool_bytes(&self, extra: usize) -> usize {
        let span = (self.domain().1 as i128 - self.domain().0 as i128 + 1).max(0) as usize;
        let domain = if gpu_triangles::dense_domain(self.domain(), self.sources.len()) {
            span
        } else {
            0
        };
        // Sum of rounded builder allocations plus query storage and fragmentation.
        self.sources.len() * 64 + domain * 24 + extra + (32 << 20)
    }
}
pub fn memory() -> Result<(usize, usize), Error> {
    let (mut free, mut total) = (0, 0);
    let status = unsafe { cuda_core::sys::cuMemGetInfo_v2(&mut free, &mut total) };
    if status != 0 {
        return Err(format!("cuMemGetInfo status {status}").into());
    }
    Ok((free, total))
}
pub fn median(values: &[f32]) -> f32 {
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}
