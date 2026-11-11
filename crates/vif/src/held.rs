//! Mutexes taken even when a thread panicked while holding one. What they guard here is plain
//! state that stays usable, and a session host must not lose its shell to one thread's panic.

use std::sync::{Mutex, MutexGuard, PoisonError};

pub trait Held<T> {
    fn held(&self) -> MutexGuard<'_, T>;
}

impl<T> Held<T> for Mutex<T> {
    fn held(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_panic_under_the_lock_leaves_it_usable() {
        let state = Arc::new(Mutex::new(1));
        let held = Arc::clone(&state);
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("on purpose");
        })
        .join();
        assert!(state.is_poisoned());
        *state.held() += 1;
        assert_eq!(*state.held(), 2);
    }
}
