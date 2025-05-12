use crate::Runtime;
use crate::scheduler::Scheduler;
use std::any::Any;
use std::fmt::{Debug, Formatter};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::task::{Context, Poll, Wake, Waker};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[repr(u8)]
pub enum TaskState {
    Pending = 0,
    Finished = 1,
    Panicked = 2,
}

impl From<u8> for TaskState {
    fn from(val: u8) -> Self {
        match val {
            0 => TaskState::Pending,
            1 => TaskState::Finished,
            2 => TaskState::Panicked,
            _ => unreachable!(),
        }
    }
}

pub(crate) enum Wait {
    Sync,
    Async(Waker),
}

pub(crate) trait ITask: Debug + Send + Sync {
    fn resume(&self, scheduler: &Arc<Scheduler>, runtime: &Runtime);

    fn on_panic(&self, err: Box<dyn Any + Send>);
}

pub(crate) trait TTask<T>: ITask {
    fn task_state(&self) -> TaskState;

    fn condvar(&self) -> &Condvar;

    fn result(&self) -> &Mutex<(Option<Wait>, Option<Result<T, Box<dyn Any + Send>>>)>;
}

pub(crate) struct TaskInner<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    this: Weak<Self>,
    future: Mutex<F>,
    state: Mutex<Option<WakeState>>,
    result: Mutex<(Option<Wait>, Option<Result<F::Output, Box<dyn Any + Send>>>)>,
    condvar: Condvar,
    task_state: AtomicU8,
}

enum WakeState {
    Scheduler(Arc<Scheduler>, Runtime),
    Waked,
}

impl<F> TaskInner<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    pub(crate) fn new(future: F) -> Arc<dyn TTask<F::Output>> {
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            future: Mutex::new(future),
            state: Mutex::new(None),
            result: Mutex::new((None, None)),
            condvar: Condvar::new(),
            task_state: AtomicU8::new(TaskState::Pending as u8),
        })
    }
}

impl<F> ITask for TaskInner<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn resume(&self, scheduler: &Arc<Scheduler>, runtime: &Runtime) {
        let this = self.this.upgrade().unwrap();
        let waker = Waker::from(this.clone());
        let mut ctx = Context::from_waker(&waker);
        let mut future_place = self.future.lock().unwrap();
        let future: Pin<&mut F> = unsafe { Pin::new_unchecked(&mut *future_place) };
        let poll = future.poll(&mut ctx);
        drop(future_place);
        match poll {
            Poll::Pending => {
                let mut state = self.state.lock().unwrap();
                match state.take() {
                    Some(WakeState::Waked) => {
                        drop(state);
                        scheduler.dispatch(this, &runtime);
                    }
                    _ => {
                        state.replace(WakeState::Scheduler(scheduler.clone(), runtime.clone()));
                    }
                };
            }
            Poll::Ready(result) => {
                self.on_result(Ok(result));
            }
        }
    }

    fn on_panic(&self, err: Box<dyn Any + Send>) {
        self.on_result(Err(err));
    }
}

impl<F> TTask<F::Output> for TaskInner<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn task_state(&self) -> TaskState {
        self.task_state.load(Relaxed).into()
    }

    fn condvar(&self) -> &Condvar {
        &self.condvar
    }

    fn result(&self) -> &Mutex<(Option<Wait>, Option<Result<F::Output, Box<dyn Any + Send>>>)> {
        &self.result
    }
}

impl<F> TaskInner<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn on_result(&self, result: Result<F::Output, Box<dyn Any + Send>>) {
        let state = if result.is_ok() {
            TaskState::Finished
        } else {
            TaskState::Panicked
        };
        let mut result_place = self.result.lock().unwrap();
        result_place.1.replace(result);
        self.task_state.store(state as u8, Relaxed);
        if let Some(ev) = result_place.0.take() {
            match ev {
                Wait::Sync => {
                    self.condvar.notify_one();
                }
                Wait::Async(waker) => {
                    waker.wake();
                }
            }
        }
    }
}

impl<F> Wake for TaskInner<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn wake(self: Arc<Self>) {
        let mut state = self.state.lock().unwrap();
        match state.take() {
            Some(WakeState::Scheduler(scheduler, runtime)) => {
                drop(state);
                scheduler.dispatch(self.clone(), &runtime);
            }
            _ => {
                *state = Some(WakeState::Waked);
            }
        };
    }
}

impl<F> Debug for TaskInner<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskInner").finish()
    }
}

pub struct Task<T> {
    pub(crate) runtime: Runtime,
    pub(crate) inner: Arc<dyn TTask<T>>,
    _p: PhantomData<T>,
}

impl<T: Send + 'static> Task<T> {
    pub fn new_in<F>(runtime: &Runtime, f: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let inner = TaskInner::new(f);
        runtime.scheduler.dispatch(inner.clone(), runtime);
        Self {
            runtime: runtime.clone(),
            inner,
            _p: PhantomData,
        }
    }

    pub fn new<F>(f: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let runtime = Runtime::current().expect("There is no runtime for the current thread");
        Self::new_in(&runtime, f)
    }

    pub fn state(&self) -> TaskState {
        self.inner.task_state()
    }
}

impl<T> Debug for Task<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task").finish()
    }
}

impl<T: Send + 'static> Task<T> {
    pub fn wait(self) -> Result<T, Box<dyn Any + Send>> {
        let scope = self.runtime.scope();
        loop {
            let mut result = if self.state() == TaskState::Pending {
                let mut result = self.inner.result().lock().unwrap();
                if self.state() == TaskState::Pending {
                    result.0.replace(Wait::Sync);
                    self.inner.condvar().wait(result).unwrap()
                } else {
                    result
                }
            } else {
                self.inner.result().lock().unwrap()
            };
            match result.1.take() {
                Some(result) => {
                    drop(scope);
                    return result;
                }
                None => continue,
            }
        }
    }
}

impl<T: Send + 'static> Future for Task<T> {
    type Output = Result<T, Box<dyn Any + Send>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut result = self.inner.result().lock().unwrap();
        match result.1.take() {
            None => {
                result.0.replace(Wait::Async(cx.waker().clone()));
                Poll::Pending
            }
            Some(r) => Poll::Ready(r),
        }
    }
}
