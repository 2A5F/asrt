use crate::task::ITask;
use crate::{Runtime, RuntimeBuilder};
use concurrent_queue::{ConcurrentQueue, PopError};
use crossbeam::utils::CachePadded;
use std::ops::Deref;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::{Arc, Mutex};
use std::thread::Thread;

#[repr(align(128))]
#[derive(Debug)]
struct Single {
    pub count: AtomicUsize,
    pub mutex: SingleMutex,
}

#[repr(align(128))]
#[derive(Debug)]
struct SingleMutex(Mutex<()>);

impl Deref for SingleMutex {
    type Target = Mutex<()>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(crate) struct Scheduler {
    min_threads: usize,
    task_queue: ConcurrentQueue<Arc<dyn ITask>>,
    thread_queue: ConcurrentQueue<(Thread, Arc<Single>)>,
    thread_id_inc: AtomicUsize,
    threads: AtomicUsize,
    running_threads: CachePadded<AtomicUsize>,
}

impl Scheduler {
    pub(crate) fn new(builder: &RuntimeBuilder) -> Arc<Self> {
        Arc::new(Self {
            min_threads: builder.min_threads.get(),
            task_queue: ConcurrentQueue::unbounded(),
            thread_queue: ConcurrentQueue::unbounded(),
            thread_id_inc: AtomicUsize::new(0),
            threads: AtomicUsize::new(0),
            running_threads: CachePadded::new(AtomicUsize::new(0)),
        })
    }

    pub(crate) fn launch(self: &Arc<Self>, runtime: &Runtime) {
        for _ in 0..self.min_threads {
            self.spawn(runtime.clone());
        }
    }

    fn spawn(self: &Arc<Self>, runtime: Runtime) {
        self.threads.fetch_add(1, Release);
        let this = self.clone();
        std::thread::Builder::new()
            .name((runtime.thread_name_fn)(
                self.thread_id_inc.fetch_add(1, Relaxed),
            ))
            .spawn(move || {
                this.running(runtime);
            })
            .unwrap();
    }

    fn running(self: &Arc<Self>, runtime: Runtime) {
        self.running_threads.fetch_add(1, Release);
        let scope = runtime.scope();
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let single = Arc::new(Single {
                count: AtomicUsize::new(0),
                mutex: SingleMutex(Mutex::new(())),
            });
            let thread = std::thread::current();
            'root: loop {
                let v = single.count.load(Acquire);
                if v == 0 {
                    let lock = single.mutex.lock().unwrap();
                    let v = single.count.load(Acquire);
                    if v == 0 {
                        self.thread_queue
                            .push((thread.clone(), single.clone()))
                            .unwrap();
                        drop(lock);
                        self.running_threads.fetch_sub(1, Release);
                        std::thread::park();
                        self.running_threads.fetch_add(1, Release);
                    }
                }
                single.count.fetch_sub(1, Release);
                'inner: loop {
                    let task = 'task: {
                        for _ in 0..100 {
                            match self.task_queue.pop() {
                                Ok(task) => {
                                    break 'task task;
                                }
                                Err(PopError::Empty) => {}
                                Err(PopError::Closed) => {
                                    break 'root;
                                }
                            }
                        }
                        continue 'root;
                    };
                    match std::panic::catch_unwind(AssertUnwindSafe(|| task.resume(self, &runtime)))
                    {
                        Ok(_) => continue 'inner,
                        Err(e) => task.on_panic(e),
                    }
                }
            }
        }));
        self.threads.fetch_sub(1, Relaxed);
        self.running_threads.fetch_sub(1, Release);
        if let Err(e) = r {
            std::panic::resume_unwind(e)
        }
        drop(scope)
    }

    pub(crate) fn dispatch(&self, task: Arc<dyn ITask>, _runtime: &Runtime) {
        self.task_queue.push(task).unwrap();
        loop {
            match self.thread_queue.pop() {
                Ok((thread, single)) => {
                    let lock = single.mutex.lock().unwrap();
                    single.count.fetch_add(1, Release);
                    thread.unpark();
                    drop(lock);
                }
                Err(PopError::Empty) => {}
                Err(PopError::Closed) => {
                    unreachable!();
                }
            }
            if self.running_threads.load(Acquire) != 0 {
                return;
            }
            // todo timeout spawn new thread
        }
    }
}
