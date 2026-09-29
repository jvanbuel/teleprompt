//! Where a take is. One value, so a take cannot be paused and sending at
//! once, or counting down while it runs.

use std::time::{Duration, Instant};

use teleprompt_gtk::take::Take;

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn a_take_runs_pauses_and_stops_with_its_time() {
    let t0 = Instant::now();
    let mut take = Take::default();
    assert!(!take.is_taking());
    take.start(t0);
    assert!(take.is_sending() && take.is_under_way());
    assert_eq!(take.elapsed(t0 + s(3)), s(3));

    assert_eq!(take.toggle_pause(t0 + s(3)), Some(true));
    assert!(!take.is_sending() && take.is_under_way());
    // Paused, the clock stands still.
    assert_eq!(take.elapsed(t0 + s(10)), s(3));

    assert_eq!(take.toggle_pause(t0 + s(10)), Some(false));
    assert_eq!(take.elapsed(t0 + s(12)), s(5));

    take.stop(t0 + s(12));
    assert!(!take.is_taking());
    // Stopped, the clock shows how long the take ran.
    assert_eq!(take.elapsed(t0 + s(60)), s(5));
}

#[test]
fn a_countdown_is_a_take_about_to_start() {
    let t0 = Instant::now();
    let mut take = Take::default();
    assert!(take.count());
    assert!(take.is_taking() && take.is_counting() && !take.is_under_way());
    // A second take waits for it.
    assert!(!take.count());
    // Nothing to pause yet.
    assert_eq!(take.toggle_pause(t0), None);
    take.start(t0);
    assert!(!take.is_counting() && take.is_sending());
}

#[test]
fn stopping_a_countdown_cancels_it() {
    let mut take = Take::default();
    take.count();
    take.stop(Instant::now());
    assert!(!take.is_taking());
}

#[test]
fn a_fresh_start_clears_the_clock() {
    let t0 = Instant::now();
    let mut take = Take::default();
    take.start(t0);
    take.stop(t0 + s(4));
    take.reset();
    assert_eq!(take.elapsed(t0 + s(9)), Duration::ZERO);
}
