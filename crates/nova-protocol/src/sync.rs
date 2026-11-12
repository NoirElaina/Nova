use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Extension trait for recovering cleanly from a poisoned `std::sync::Mutex`.
///
/// Acquiring a lock after another thread panicked should recover the protected
/// state rather than cascading panics across workers.
pub trait MutexExt<T: ?Sized> {
    /// Acquires the mutex guard, recovering the underlying value if poisoned.
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> MutexExt<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Extension trait for recovering cleanly from a poisoned `std::sync::RwLock`.
pub trait RwLockExt<T: ?Sized> {
    /// Acquires a shared read guard, recovering the underlying value if poisoned.
    fn read_unpoisoned(&self) -> RwLockReadGuard<'_, T>;
    /// Acquires an exclusive write guard, recovering the underlying value if poisoned.
    fn write_unpoisoned(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    fn read_unpoisoned(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_unpoisoned(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::{MutexExt, RwLockExt};
    use std::sync::{Arc, Mutex, RwLock};

    #[test]
    fn mutex_recovers_after_poison() {
        let m = Arc::new(Mutex::new(41_i32));
        let m2 = Arc::clone(&m);
        let handle = std::thread::spawn(move || {
            let mut guard = m2.lock().unwrap();
            *guard = 42;
            panic!("intentional panic to poison mutex");
        });
        assert!(handle.join().is_err());
        assert_eq!(*m.lock_unpoisoned(), 42);
    }

    #[test]
    fn rwlock_recovers_after_poison() {
        let lock = Arc::new(RwLock::new(String::from("before")));
        let lock2 = Arc::clone(&lock);
        let handle = std::thread::spawn(move || {
            let mut guard = lock2.write().unwrap();
            guard.clear();
            guard.push_str("after");
            panic!("intentional panic to poison rwlock");
        });
        assert!(handle.join().is_err());
        assert_eq!(&*lock.read_unpoisoned(), "after");
        lock.write_unpoisoned().push('!');
        assert_eq!(&*lock.read_unpoisoned(), "after!");
    }
}
