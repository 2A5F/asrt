use crate::scheduler::Scheduler;
use crate::task::*;
use std::cell::{Cell, RefCell};
use std::fmt::{Debug, Formatter};
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::sync::{Arc, LazyLock, MutexGuard};

#[derive(Clone)]
pub struct Runtime {
    pub(crate) scheduler: Arc<Scheduler>,
    pub(crate) thread_name_fn: TheadNameFn,
}

impl Debug for Runtime {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").finish()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self::builder().build()
    }

    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }
}

pub type TheadNameFn = Arc<dyn Fn(usize) -> String + Send + Sync + 'static>;

static DEFAULT_THEAD_NAME_FN: LazyLock<TheadNameFn> =
    LazyLock::new(|| Arc::new(|id| format!("async runtime threads {id}")));

pub struct RuntimeBuilder {
    pub(crate) min_threads: NonZeroUsize,
    pub(crate) thread_name_fn: TheadNameFn,
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            min_threads: std::thread::available_parallelism()
                .unwrap_or(NonZeroUsize::new(1).unwrap()),
            thread_name_fn: DEFAULT_THEAD_NAME_FN.clone(),
        }
    }

    pub fn min_threads(&mut self, n: NonZeroUsize) -> &mut Self {
        self.min_threads = n;
        self
    }

    pub fn build(&self) -> Runtime {
        let rt = Runtime {
            scheduler: Scheduler::new(&self),
            thread_name_fn: self.thread_name_fn.clone(),
        };
        rt.scheduler.launch(&rt);
        rt
    }
}

impl Runtime {
    pub fn run<F>(&self, future: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Task::new_in(self, future)
    }
}

thread_local! {
    static CUURRENT_RUNTIME: RefCell<Option<Runtime>> = RefCell::new(None);
}

impl Runtime {
    pub fn scope(&self) -> RuntimeScope {
        RuntimeScope::enter(self.clone())
    }

    pub fn current() -> Option<Runtime> {
        CUURRENT_RUNTIME.with_borrow(|current| current.clone())
    }
}

pub struct RuntimeScope {
    last: Option<Runtime>,
    _p: PhantomData<(Cell<()>, MutexGuard<'static, ()>)>,
}

impl RuntimeScope {
    fn enter(cur: Runtime) -> Self {
        let last = CUURRENT_RUNTIME.replace(Some(cur));
        Self {
            last,
            _p: PhantomData,
        }
    }
}

impl Drop for RuntimeScope {
    fn drop(&mut self) {
        CUURRENT_RUNTIME.replace(self.last.take());
    }
}

impl Debug for RuntimeScope {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeScope").finish()
    }
}
