//! One answer to a poisoned lock, for every crate that holds a mutex.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Locks `m`, and carries on if a thread panicked while holding it.
///
/// A poisoned mutex says that some thread panicked mid-update, not that the
/// data is unusable, and the callers here hold plain state (a cache, the last
/// reading, a log) whose next write replaces it. Passing the panic on to
/// every later caller would turn one failed request into a dead server.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_lock_is_taken_even_after_a_holder_panicked() {
        let m = Arc::new(Mutex::new(1));
        let held = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let mut n = held.lock().unwrap();
            *n = 2;
            panic!("while holding the lock");
        })
        .join();
        assert!(m.is_poisoned());
        assert_eq!(*lock(&m), 2);
    }
}
