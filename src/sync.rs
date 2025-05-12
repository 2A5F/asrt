use std::fmt::{Debug, Formatter};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Acquire, Release};
use std::sync::{Condvar, Mutex};

pub struct Fence {
    value: AtomicUsize,
    condvar: Condvar,
    mutex: Mutex<()>,
}

impl Fence {
    pub fn new() -> Self {
        Self {
            value: AtomicUsize::new(0),
            condvar: Condvar::new(),
            mutex: Mutex::new(()),
        }
    }
}

impl Debug for Fence {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Fence").field(&self.value()).finish()
    }
}

impl Fence {
    pub fn value(&self) -> usize {
        self.value.load(Acquire)
    }

    pub fn single(&self, value: usize) {
        self.value.store(value, Release);
        let lock = self.mutex.lock();
        self.condvar.notify_all();
        drop(lock);
    }

    pub fn wait(&self, value: usize) {
        drop(
            self.condvar
                .wait_while(self.mutex.lock().unwrap(), |_| self.value() < value)
                .unwrap(),
        );
    }
}
