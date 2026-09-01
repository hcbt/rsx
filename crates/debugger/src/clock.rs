//! Guest crystal versus wall time. The DAC is a speaker, not this clock.

use std::time::Duration;

use rsx_machine::{cycles_in_nanos, ntsc_vblank_hz, CPU_HZ};

/// Wall time of one SPU sample (768 master cycles). In-sync wake, not a guessed floor.
pub fn sample_period() -> Duration {
    let ns = (768u128 * 1_000_000_000 + u128::from(CPU_HZ) - 1) / u128::from(CPU_HZ);
    Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Guest is behind wall: run until [`target_cycles`].
    Run,
    /// Guest is at or ahead of wall: present and sleep this long.
    Wait(Duration),
}

/// Guest cycle deadline for `elapsed` of wall time since `origin_cycles`.
pub fn target_cycles(origin_cycles: u64, elapsed: Duration) -> u64 {
    origin_cycles.saturating_add(cycles_in_nanos(elapsed.as_nanos()))
}

/// How long wall time must advance before `guest_cycles` is due.
pub fn wait_for_wall(guest_cycles: u64, origin_cycles: u64, elapsed: Duration) -> Option<Duration> {
    let guest_elapsed = u128::from(guest_cycles.saturating_sub(origin_cycles));
    let due_ns = guest_elapsed * 1_000_000_000 / u128::from(CPU_HZ);
    let elapsed_ns = elapsed.as_nanos();
    if due_ns > elapsed_ns {
        Some(Duration::from_nanos(
            u64::try_from(due_ns - elapsed_ns).unwrap_or(u64::MAX),
        ))
    } else {
        None
    }
}

/// Guest cycles and vblanks per wall second, versus the crystal and NTSC.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostPace {
    pub hz: f64,
    pub fps: f64,
    /// 1.0 means the guest advanced at [`CPU_HZ`].
    pub of_crystal: f64,
    /// 1.0 means vblanks arrived at [`ntsc_vblank_hz`].
    pub of_ntsc: f64,
}

impl HostPace {
    pub fn behind(self) -> bool {
        self.of_crystal < 0.95
    }

    pub fn line(self) -> String {
        format!(
            "pace clock={:.2}/{:.2}MHz ({:.0}%) fps={:.1}/{:.2} ({:.0}%)",
            self.hz / 1_000_000.0,
            CPU_HZ as f64 / 1_000_000.0,
            self.of_crystal * 100.0,
            self.fps,
            ntsc_vblank_hz(),
            self.of_ntsc * 100.0,
        )
    }
}

/// `delta_cycles` and `delta_vblanks` over `elapsed` of wall time.
pub fn measure(delta_cycles: u64, delta_vblanks: u64, elapsed: Duration) -> Option<HostPace> {
    let secs = elapsed.as_secs_f64();
    if !(secs > 0.0) {
        return None;
    }
    let hz = delta_cycles as f64 / secs;
    let fps = delta_vblanks as f64 / secs;
    Some(HostPace {
        hz,
        fps,
        of_crystal: hz / CPU_HZ as f64,
        of_ntsc: fps / ntsc_vblank_hz(),
    })
}

pub fn pace(guest_cycles: u64, origin_cycles: u64, elapsed: Duration) -> Pace {
    if guest_cycles < target_cycles(origin_cycles, elapsed) {
        Pace::Run
    } else {
        let wait = wait_for_wall(guest_cycles, origin_cycles, elapsed).unwrap_or(Duration::ZERO);
        Pace::Wait(if wait.is_zero() {
            sample_period()
        } else {
            wait
        })
    }
}

/// Host time spent on one Debugger present. ADR 0004 copies the Display area
/// once per guest vblank; a tight egui spin must not starve `run_until_cycle`.
pub const RUN_SLICE: Duration = Duration::from_millis(32);

/// Floor for `request_repaint_after` when the guest is at/ahead of wall.
/// `pace` waits one SPU sample (~23 µs); waking egui that often rebuilds the
/// UI at tens of kHz and starves the Machine (Spyro title 89–94%).
pub fn present_wait(wait: Duration) -> Duration {
    let vblank_ns = (rsx_machine::CYCLES_PER_LINE * u64::from(rsx_machine::LINES_PER_FRAME))
        * 1_000_000_000
        / CPU_HZ;
    let min = Duration::from_nanos(vblank_ns);
    if wait < min {
        min
    } else {
        wait
    }
}

/// Copy the Display area into the wgpu texture only when the guest vblank
/// advanced (ADR 0004).
pub fn display_needs_present(uploaded_vblank: u64, guest_vblank: u64) -> bool {
    guest_vblank != uploaded_vblank
}

/// GPR / I/O log rebuild while `Pace::Run` (behind) starves the next
/// `run_until_cycle` slice. Spyro title is ~99% headless; that inspect
/// paint is the windowed drop.
pub fn inspect_needs_paint(pace: Pace) -> bool {
    !matches!(pace, Pace::Run)
}

/// Run the guest until it is at/ahead of wall or `budget` of host time in
/// `run_to` has elapsed. A present slice that returns after one tiny `Run`
/// starves catch-up (Spyro title ~121% headless becomes 89–94% windowed).
pub fn run_slice(
    origin_cycles: u64,
    mut wall: impl FnMut() -> Duration,
    mut guest_cycles: impl FnMut() -> u64,
    budget: Duration,
    mut host_dt: impl FnMut() -> Duration,
    mut run_to: impl FnMut(u64),
) -> Pace {
    loop {
        let p = pace(guest_cycles(), origin_cycles, wall());
        match p {
            Pace::Wait(_) => return p,
            Pace::Run => {
                if host_dt() >= budget {
                    return Pace::Run;
                }
                run_to(target_cycles(origin_cycles, wall()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_wall_second_is_the_crystal() {
        assert_eq!(target_cycles(0, Duration::from_secs(1)), 33_868_800);
        assert_eq!(target_cycles(100, Duration::from_secs(1)), 100 + 33_868_800);
    }

    #[test]
    fn guest_behind_wall_runs() {
        assert_eq!(pace(0, 0, Duration::from_secs(1)), Pace::Run);
    }

    #[test]
    fn guest_ahead_of_wall_waits_the_difference() {
        assert_eq!(
            pace(33_868_800, 0, Duration::ZERO),
            Pace::Wait(Duration::from_secs(1))
        );
    }

    #[test]
    fn in_sync_waits_one_spu_sample() {
        let sample = sample_period();
        assert_eq!(sample, Duration::from_nanos(22_676));
        assert_eq!(pace(0, 0, Duration::ZERO), Pace::Wait(sample));
        assert_eq!(
            pace(33_868_800, 0, Duration::from_secs(1)),
            Pace::Wait(sample)
        );
    }

    #[test]
    fn wait_for_wall_is_none_when_behind_or_equal() {
        assert!(wait_for_wall(0, 0, Duration::from_secs(1)).is_none());
        assert!(wait_for_wall(33_868_800, 0, Duration::from_secs(1)).is_none());
    }

    #[test]
    fn measure_at_the_crystal_is_unity() {
        let p = measure(CPU_HZ, 0, Duration::from_secs(1)).unwrap();
        assert!((p.hz - CPU_HZ as f64).abs() < 1.0);
        assert!((p.of_crystal - 1.0).abs() < 1e-9);
        assert!(!p.behind());
        assert!(p.line().contains("33.87"));
    }

    #[test]
    fn measure_half_speed_is_behind() {
        let p = measure(CPU_HZ / 2, 30, Duration::from_secs(1)).unwrap();
        assert!((p.of_crystal - 0.5).abs() < 1e-9);
        assert!(p.behind());
        assert!((p.fps - 30.0).abs() < 1e-9);
        assert!(p.of_ntsc < 0.6);
    }

    #[test]
    fn measure_rejects_zero_elapsed() {
        assert!(measure(CPU_HZ, 60, Duration::ZERO).is_none());
    }

    #[test]
    fn present_wait_is_at_least_one_vblank_not_one_spu_sample() {
        let sample = sample_period();
        let w = present_wait(sample);
        assert!(
            w >= Duration::from_millis(15),
            "egui must not wake every SPU sample ({sample:?} → {w:?})"
        );
        let long = Duration::from_millis(50);
        assert_eq!(present_wait(long), long);
    }

    #[test]
    fn display_needs_present_only_when_vblank_advanced() {
        assert!(!display_needs_present(12, 12));
        assert!(display_needs_present(12, 13));
        assert!(display_needs_present(0, 1));
    }

    #[test]
    fn inspect_ui_is_skipped_while_behind() {
        assert!(
            !inspect_needs_paint(Pace::Run),
            "32 GPR labels + log must not rebuild on a catch-up slice"
        );
        assert!(inspect_needs_paint(Pace::Wait(Duration::from_millis(16))));
    }

    #[test]
    fn run_slice_stops_at_wait_when_guest_catches_wall() {
        let guest = std::cell::Cell::new(0u64);
        let wall = Duration::from_millis(10);
        let p = run_slice(
            0,
            || wall,
            || guest.get(),
            Duration::from_secs(1),
            || Duration::ZERO,
            |target| guest.set(target),
        );
        assert!(
            matches!(p, Pace::Wait(_)),
            "one catch-up to the wall must Wait, not spin Run"
        );
        assert!(guest.get() >= target_cycles(0, wall));
    }

    #[test]
    fn run_slice_keeps_running_until_budget_when_still_behind() {
        let guest = std::cell::Cell::new(0u64);
        let host = std::cell::Cell::new(Duration::ZERO);
        let steps = std::cell::Cell::new(0u32);
        let wall = Duration::from_secs(1);
        let p = run_slice(
            0,
            || wall,
            || guest.get(),
            Duration::from_millis(5),
            || host.get(),
            |_target| {
                guest.set(guest.get() + 1);
                steps.set(steps.get() + 1);
                host.set(host.get() + Duration::from_millis(1));
            },
        );
        assert_eq!(p, Pace::Run, "still behind after the present budget");
        assert!(
            steps.get() >= 5,
            "must keep calling run_to until budget, not once (steps={})",
            steps.get()
        );
    }
}
