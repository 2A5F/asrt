use crate::task::ITask;
use crate::{Runtime, RuntimeBuilder};
use concurrent_queue::{ConcurrentQueue, PopError};
use crossbeam::utils::CachePadded;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering::{AcqRel, Acquire};
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::{Arc, Condvar, Mutex};

pub(crate) struct Scheduler {
    min_threads: usize,
    task_queue: Vec<ConcurrentQueue<Arc<dyn ITask>>>,
    cur_queue: CachePadded<AtomicU64>,
    thread_ctrl: CachePadded<(Condvar, Mutex<()>)>,
    thread_id_inc: AtomicUsize,
    threads: AtomicUsize,
    running_threads: CachePadded<AtomicUsize>,
}

impl Scheduler {
    pub(crate) fn new(builder: &RuntimeBuilder) -> Arc<Self> {
        Arc::new(Self {
            min_threads: builder.min_threads.get(),
            task_queue: (0..builder.min_threads.get())
                .map(|_| ConcurrentQueue::unbounded())
                .collect(),
            cur_queue: CachePadded::new(AtomicU64::new(0)),
            thread_ctrl: CachePadded::new((Condvar::new(), Mutex::new(()))),
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
        self.threads.fetch_add(1, AcqRel);
        let this = self.clone();
        std::thread::Builder::new()
            .name((runtime.thread_name_fn)(
                self.thread_id_inc.fetch_add(1, AcqRel),
            ))
            .spawn(move || {
                this.running(runtime);
            })
            .unwrap();
    }

    fn running(self: &Arc<Self>, runtime: Runtime) {
        self.running_threads.fetch_add(1, AcqRel);
        let scope = runtime.scope();
        let max_try_count = self.task_queue.len() * 16;
        let mut cur_queue = 0;
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
            'root: loop {
                let lock = self.thread_ctrl.1.lock().unwrap();
                self.running_threads.fetch_sub(1, AcqRel);
                drop(self.thread_ctrl.0.wait(lock).unwrap());
                self.running_threads.fetch_add(1, AcqRel);
                'inner: loop {
                    let task = 'task: {
                        for _ in 0..max_try_count {
                            let queue = cur_queue;
                            cur_queue += 1;
                            let queue = (queue % self.task_queue.len() as u64) as usize;
                            match self.task_queue[queue].pop() {
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
        self.threads.fetch_sub(1, AcqRel);
        self.running_threads.fetch_sub(1, AcqRel);
        if let Err(e) = r {
            std::panic::resume_unwind(e)
        }
        drop(scope)
    }

    pub(crate) fn dispatch(&self, task: Arc<dyn ITask>, _runtime: &Runtime) {
        let queue = (self.cur_queue.fetch_add(1, AcqRel) % self.task_queue.len() as u64) as usize;
        self.task_queue[queue].push(task).unwrap();
        if self.running_threads.load(Acquire) == 0 {
            self.thread_ctrl.0.notify_one();
        }
        loop {
            if self.running_threads.load(Acquire) != 0 {
                return;
            }
            let lock = self.thread_ctrl.1.lock().unwrap();
            self.thread_ctrl.0.notify_all();
            drop(lock);
            // todo timeout spawn new thread
        }
    }
}
