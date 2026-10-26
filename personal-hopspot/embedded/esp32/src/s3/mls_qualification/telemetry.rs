use portable_atomic::{AtomicU64, AtomicUsize, Ordering};

static MIN_INTERNAL: AtomicUsize = AtomicUsize::new(usize::MAX);
static MIN_EXTERNAL: AtomicUsize = AtomicUsize::new(usize::MAX);
static SAMPLED_PEAK: AtomicUsize = AtomicUsize::new(0);
static LAST_SAMPLE_US: AtomicU64 = AtomicU64::new(0);
static MAX_SAMPLE_GAP_US: AtomicU64 = AtomicU64::new(0);

// Only the periodic Embassy observer calls this. Worker phase samples must not
// hide a delay in the observer's nominal 10 ms schedule.
pub(super) fn sampler_tick() {
    let now = embassy_time::Instant::now().as_micros();
    let previous = LAST_SAMPLE_US.swap(now, Ordering::Relaxed);
    if previous != 0 {
        MAX_SAMPLE_GAP_US.fetch_max(now.saturating_sub(previous), Ordering::Relaxed);
    }
    update();
}

pub(super) fn update() {
    let stats = esp_alloc::HEAP.stats();
    let mut internal = 0;
    let mut external = 0;
    for region in stats.region_stats.iter().flatten() {
        if region
            .capabilities
            .contains(esp_alloc::MemoryCapability::Internal)
        {
            internal += region.free;
        }
        if region
            .capabilities
            .contains(esp_alloc::MemoryCapability::External)
        {
            external += region.free;
        }
    }
    MIN_INTERNAL.fetch_min(internal, Ordering::Relaxed);
    MIN_EXTERNAL.fetch_min(external, Ordering::Relaxed);
    SAMPLED_PEAK.fetch_max(stats.current_usage, Ordering::Relaxed);
}

pub(super) fn sample(label: &str) {
    update();
    let stack_pointer: usize;
    // SAFETY: Reading the Xtensa stack register changes no machine state or memory.
    unsafe {
        core::arch::asm!("mov {stack_pointer}, a1", stack_pointer = out(reg) stack_pointer, options(nomem, nostack))
    };
    log::info!(
        "MLS stack phase={label} sp={stack_pointer:#x} aligned16={}",
        stack_pointer % 16 == 0
    );
    let stats = esp_alloc::HEAP.stats();
    let internal = esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::Internal.into());
    let external = esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::External.into());
    log::info!("MLS memory phase={label} t_ms={} current={} boot_peak={} sampled_peak={} internal_free={internal} external_free={external} min_internal={} min_external={} core1_heartbeat={} sample_gap_max_us={}",
        embassy_time::Instant::now().as_millis(), stats.current_usage, stats.max_usage,
        SAMPLED_PEAK.load(Ordering::Relaxed), MIN_INTERNAL.load(Ordering::Relaxed), MIN_EXTERNAL.load(Ordering::Relaxed),
        super::super::CORE_ONE_HEARTBEAT.load(Ordering::Relaxed), MAX_SAMPLE_GAP_US.load(Ordering::Relaxed));
}
