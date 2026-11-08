//! Millisecond pacing for the CLI's audio loops, done the way audio apps do it.
//!
//! `std::thread::sleep(1 ms)` can last a whole Windows timer tick (up to ~15.6 ms),
//! longer than a 20 ms lead allows under load (VM run a3: the Ring ran dry 23
//! times in 60 s). On Windows this uses a high-resolution waitable timer (true ~1 ms
//! waits, without raising the whole machine's timer resolution as timeBeginPeriod
//! would) and registers the thread with MMCSS as "Pro Audio" for audio priority.
//! Elsewhere it falls back to plain sleeps.

use std::time::{Duration, Instant};

/// Sleeps in short, accurate steps and keeps the longest gap between steps.
pub struct Pacer {
    #[cfg(windows)]
    timer: Option<windows::Win32::Foundation::HANDLE>,
    #[cfg(windows)]
    mmcss: Option<windows::Win32::Foundation::HANDLE>,
    last: Option<Instant>,
    longest_gap: Duration,
    gaps_over: u64,
    limit: Duration,
}

impl Pacer {
    /// A pacer for the current thread; gaps longer than `limit` are counted.
    pub fn new(limit: Duration) -> Pacer {
        Pacer {
            #[cfg(windows)]
            timer: win::high_resolution_timer(),
            #[cfg(windows)]
            mmcss: win::join_pro_audio(),
            last: None,
            longest_gap: Duration::ZERO,
            gaps_over: 0,
            limit,
        }
    }

    /// Marks one pass of the loop: measures the gap since the previous pass.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last {
            let gap = now - last;
            self.longest_gap = self.longest_gap.max(gap);
            if gap > self.limit {
                self.gaps_over += 1;
            }
        }
        self.last = Some(now);
    }

    /// Waits about one millisecond.
    pub fn sleep_1ms(&self) {
        #[cfg(windows)]
        if let Some(timer) = self.timer
            && win::wait(timer, Duration::from_millis(1))
        {
            return;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "the fallback where no high-resolution timer exists"
        )]
        std::thread::sleep(Duration::from_millis(1));
    }

    /// The longest gap between passes so far, and how many exceeded the limit.
    pub fn gaps(&self) -> (Duration, u64) {
        (self.longest_gap, self.gaps_over)
    }

    /// Whether the high-resolution timer and MMCSS registration are in use.
    pub fn describe(&self) -> &'static str {
        #[cfg(windows)]
        return match (self.timer.is_some(), self.mmcss.is_some()) {
            (true, true) => "high-resolution timer, MMCSS Pro Audio",
            (true, false) => "high-resolution timer (MMCSS unavailable)",
            (false, true) => "plain sleeps (no high-resolution timer), MMCSS Pro Audio",
            (false, false) => "plain sleeps",
        };
        #[cfg(not(windows))]
        return "plain sleeps";
    }
}

#[cfg(windows)]
impl Drop for Pacer {
    fn drop(&mut self) {
        win::release(self.timer.take(), self.mmcss.take());
    }
}

#[cfg(windows)]
mod win {
    use std::time::Duration;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, SetWaitableTimer,
        TIMER_ALL_ACCESS, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, w};

    pub fn high_resolution_timer() -> Option<HANDLE> {
        // SAFETY: creates an unnamed timer owned by the Pacer.
        unsafe {
            CreateWaitableTimerExW(
                None,
                PCWSTR::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
        }
        .ok()
    }

    pub fn join_pro_audio() -> Option<HANDLE> {
        let mut index = 0u32;
        // SAFETY: registers the calling thread; reverted on Drop.
        unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut index) }.ok()
    }

    /// Waits `duration` on the timer; false if the timer couldn't be used.
    pub fn wait(timer: HANDLE, duration: Duration) -> bool {
        let due = -((duration.as_nanos() / 100) as i64); // relative, 100 ns units
        // SAFETY: the timer is open for the Pacer's lifetime.
        let armed = unsafe { SetWaitableTimer(timer, &due, 0, None, None, false) }.is_ok();
        // SAFETY: as above.
        armed && unsafe { WaitForSingleObject(timer, 100) } == WAIT_OBJECT_0
    }

    pub fn release(timer: Option<HANDLE>, mmcss: Option<HANDLE>) {
        if let Some(mmcss) = mmcss {
            // SAFETY: registered by `join_pro_audio`, reverted once.
            let _ = unsafe { AvRevertMmThreadCharacteristics(mmcss) };
        }
        if let Some(timer) = timer {
            // SAFETY: created by `high_resolution_timer`, closed once.
            let _ = unsafe { CloseHandle(timer) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_pass_measures_no_gap() {
        let mut pacer = Pacer::new(Duration::from_millis(10));

        pacer.tick();

        assert_eq!(pacer.gaps(), (Duration::ZERO, 0));
    }

    /// Not timing-sensitive: a sleep lasts at least as long as asked, and two
    /// passes back to back are far under a 10 ms limit.
    #[test]
    fn a_gap_over_the_limit_is_counted_and_kept_as_the_longest() {
        let mut pacer = Pacer::new(Duration::from_millis(10));
        pacer.tick();
        #[expect(
            clippy::disallowed_methods,
            reason = "the test needs a gap of at least 30 ms; a long sleep is fine"
        )]
        std::thread::sleep(Duration::from_millis(30));
        pacer.tick();
        pacer.tick();
        pacer.tick();

        let (longest, over) = pacer.gaps();

        // One gap over the limit and two far under it: only the long one counts.
        assert!(longest >= Duration::from_millis(30), "{longest:?}");
        assert_eq!(over, 1);
    }

    #[test]
    fn a_step_waits_and_the_pacing_is_described() {
        let pacer = Pacer::new(Duration::from_millis(10));
        let start = Instant::now();

        pacer.sleep_1ms();

        assert!(start.elapsed() >= Duration::from_micros(500));
        // Windows 10 1803 and later always provide the high-resolution timer.
        #[cfg(windows)]
        assert!(
            pacer.describe().starts_with("high-resolution timer"),
            "{}",
            pacer.describe()
        );
        #[cfg(not(windows))]
        assert_eq!(pacer.describe(), "plain sleeps");
    }

    /// Timing-dependent, so opt-in: `cargo test -p viaduct-cli -- --ignored --nocapture`.
    /// On Windows the high-resolution timer should keep 200 steps near 200 ms,
    /// where plain 1 ms sleeps can take up to a timer tick (~15.6 ms) each.
    #[test]
    #[ignore]
    fn steps_take_about_a_millisecond() {
        let mut pacer = Pacer::new(Duration::from_millis(5));
        let start = Instant::now();
        for _ in 0..200 {
            pacer.tick();
            pacer.sleep_1ms();
        }
        let (longest, over) = pacer.gaps();
        let average = start.elapsed().as_secs_f64() * 1000.0 / 200.0;
        println!(
            "{}: average step {average:.2} ms, longest {:.2} ms, {over} over 5 ms",
            pacer.describe(),
            longest.as_secs_f64() * 1000.0
        );

        assert!(average < 3.0, "average step {average:.2} ms");
    }
}
