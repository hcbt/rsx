//! Guest crystal versus wall time. The DAC is a speaker, not this clock.

use std::time::Duration;

use rsx_machine::{cycles_in_nanos, CPU_HZ};

/// Host wake-up floor so egui does not spin when guest and wall are in sync.
pub const QUANTUM: Duration = Duration::from_millis(4);

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

pub fn pace(guest_cycles: u64, origin_cycles: u64, elapsed: Duration) -> Pace {
    if guest_cycles < target_cycles(origin_cycles, elapsed) {
        Pace::Run
    } else {
        let wait = wait_for_wall(guest_cycles, origin_cycles, elapsed)
            .unwrap_or(Duration::ZERO)
            .max(QUANTUM);
        Pace::Wait(wait)
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
    fn in_sync_waits_a_quantum_instead_of_spinning() {
        assert_eq!(pace(0, 0, Duration::ZERO), Pace::Wait(QUANTUM));
        assert_eq!(
            pace(33_868_800, 0, Duration::from_secs(1)),
            Pace::Wait(QUANTUM)
        );
    }

    #[test]
    fn wait_for_wall_is_none_when_behind_or_equal() {
        assert!(wait_for_wall(0, 0, Duration::from_secs(1)).is_none());
        assert!(wait_for_wall(33_868_800, 0, Duration::from_secs(1)).is_none());
    }
}
