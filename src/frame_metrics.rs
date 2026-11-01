//! Opt-in measurement of real swapchain submissions, not an offscreen FPS estimate.
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Sample {
    at_ms: f64,
    render_ms: f64,
    acquire_ms: f64,
}

pub struct Capture {
    start: Instant,
    samples: Vec<Sample>,
    dropped: usize,
    occlusion_events: usize,
}

impl Capture {
    pub fn new() -> Self {
        Self { start: Instant::now(), samples: Vec::with_capacity(8192), dropped: 0, occlusion_events: 0 }
    }

    pub fn occluded(&mut self, occluded: bool) {
        if occluded { self.occlusion_events += 1; }
    }

    pub fn record(&mut self, now: Instant, render_ms: f64, acquire_ms: f64) {
        let at_ms = now.duration_since(self.start).as_secs_f64() * 1000.0;
        // Exclude glyph-cache and swapchain warmup for each scenario.
        if at_ms < 1000.0 { return; }
        if self.samples.len() == self.samples.capacity() {
            self.dropped += 1;
            return;
        }
        self.samples.push(Sample { at_ms, render_ms, acquire_ms });
    }

    pub fn report(&self, label: &str, input_hz: u32) -> String {
        let intervals: Vec<f64> = self.samples.windows(2).map(|s| s[1].at_ms - s[0].at_ms).collect();
        let span = self.samples.first().zip(self.samples.last()).map_or(0.0, |(a, b)| b.at_ms - a.at_ms);
        let fps = if span > 0.0 { intervals.len() as f64 * 1000.0 / span } else { 0.0 };
        let mut intervals_sorted = intervals.clone();
        intervals_sorted.sort_unstable_by(f64::total_cmp);
        let mut render: Vec<_> = self.samples.iter().map(|s| s.render_ms).collect();
        render.sort_unstable_by(f64::total_cmp);
        let mut acquire: Vec<_> = self.samples.iter().map(|s| s.acquire_ms).collect();
        acquire.sort_unstable_by(f64::total_cmp);
        let above_25 = intervals.iter().filter(|&&ms| ms > 25.0).count();
        let late = intervals.iter().filter(|&&ms| ms > 1500.0 / input_hz.max(1) as f64).count();
        format!(concat!("{{\"scenario\":\"{}\",\"input_hz\":{},\"valid\":{},\"occlusion_events\":{},\"frames\":{},\"sample_ms\":{:.3},",
            "\"submission_fps\":{:.3},\"interval_p50_ms\":{:.3},\"interval_p95_ms\":{:.3},\"interval_p99_ms\":{:.3},",
            "\"interval_max_ms\":{:.3},\"render_p95_ms\":{:.3},\"acquire_p95_ms\":{:.3},\"late_intervals\":{},\"intervals_over_25ms\":{},\"dropped_samples\":{}}}"),
            label, input_hz, self.occlusion_events == 0 && intervals.len() > 1 && self.dropped == 0, self.occlusion_events, self.samples.len(), span, fps,
            percentile(&intervals_sorted, 0.50), percentile(&intervals_sorted, 0.95), percentile(&intervals_sorted, 0.99),
            percentile(&intervals_sorted, 1.0), percentile(&render, 0.95), percentile(&acquire, 0.95), late, above_25, self.dropped)
    }
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    if sorted.is_empty() { return 0.0; }
    sorted[((sorted.len() as f64 * quantile).ceil() as usize).saturating_sub(1).min(sorted.len() - 1)]
}

pub struct ScrollRun {
    pub scenario: usize,
    pub start: Instant,
    pub next: Instant,
    pub ticks: u64,
}

impl ScrollRun {
    pub const SCENARIOS: [(&str, u32, f64); 3] = [
        ("slow_2px_120hz", 120, 2.0),
        ("fast_12px_120hz", 120, 12.0),
        ("slow_2px_60hz", 60, 2.0),
    ];
    pub fn new() -> Self {
        let start = Instant::now();
        Self { scenario: 0, start, next: start, ticks: 0 }
    }
    pub fn due(&mut self, now: Instant) -> bool {
        if now < self.next { return false; }
        self.ticks += 1;
        // Skip missed input ticks instead of creating a catch-up burst.
        let hz = Self::SCENARIOS[self.scenario].1 as f64;
        let tick = (now.duration_since(self.start).as_secs_f64() * hz).floor() + 1.0;
        self.next = self.start + Duration::from_secs_f64(tick / hz);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_intervals_include_stalls_and_skip_warmup() {
        let mut c = Capture::new();
        let start = c.start;
        for ms in [100, 1000, 1010, 1020, 1060] {
            c.record(start + Duration::from_millis(ms), 2.0, 1.0);
        }
        let report = c.report("test", 120);
        assert!(report.contains("\"frames\":4"));
        assert!(report.contains("\"submission_fps\":50.000"));
        assert!(report.contains("\"interval_p99_ms\":40.000"));
        assert!(report.contains("\"intervals_over_25ms\":1"));
        assert!(report.contains("\"late_intervals\":1"));
        assert!(report.contains("\"valid\":true"));
        c.occluded(true);
        assert!(c.report("covered", 120).contains("\"valid\":false"));
        assert!(Capture::new().report("empty", 60).contains("\"submission_fps\":0.000"));
    }
    #[test]
    fn missed_input_ticks_do_not_burst() {
        let mut run = ScrollRun::new();
        let now = run.start + Duration::from_millis(100);
        assert!(run.due(now));
        assert!(!run.due(now));
        assert!(run.next > now);
    }
    #[test]
    fn capture_records_without_allocating() {
        let mut c = Capture::new();
        let now = c.start + Duration::from_secs(2);
        let allocations = crate::test_alloc::allocations(|| {
            for _ in 0..10_000 { c.record(now, 1.0, 0.5); }
        });
        assert_eq!(allocations, 0);
        assert_eq!(c.dropped, 1808);
    }
}
