use std::fmt::{Debug, Formatter};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

pub fn async_callback<F: FnOnce(Then<R>) -> R, R>(f: F) -> CallbackFuture<F, R> {
    CallbackFuture::new(f)
}

enum CallbackFutureInner<F, R> {
    F(F),
    R(Arc<Mutex<Option<R>>>),
}

pub struct CallbackFuture<F, R> {
    inner: Option<CallbackFutureInner<F, R>>,
}

impl<F, R> CallbackFuture<F, R>
where
    F: FnOnce(Then<R>) -> R,
{
    pub fn new(f: F) -> Self {
        Self {
            inner: Some(CallbackFutureInner::F(f)),
        }
    }
}

pub struct Then<R>(Arc<Mutex<Option<R>>>, Waker);

impl<R> Debug for Then<R> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Then").finish()
    }
}

impl<R> Then<R> {
    pub fn then(self, r: R) {
        self.0.lock().unwrap().replace(r);
        self.1.wake();
    }
}

impl<F, R> Unpin for CallbackFuture<F, R> {}

impl<F, R> Debug for CallbackFuture<F, R> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackFuture").finish()
    }
}

impl<F, R> Future for CallbackFuture<F, R>
where
    F: FnOnce(Then<R>) -> R,
{
    type Output = R;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let inner = this.inner.take().unwrap();
        match inner {
            CallbackFutureInner::F(f) => {
                let then = Arc::new(Mutex::new(None));
                f(Then(then.clone(), cx.waker().clone()));
                let mut r = then.lock().unwrap();
                if r.is_some() {
                    return Poll::Ready(r.take().unwrap());
                }
                drop(r);
                this.inner.replace(CallbackFutureInner::R(then));
                Poll::Pending
            }
            CallbackFutureInner::R(then) => {
                let mut r = then.lock().unwrap();
                if r.is_some() {
                    return Poll::Ready(r.take().unwrap());
                }
                drop(r);
                this.inner.replace(CallbackFutureInner::R(then));
                Poll::Pending
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct YieldFuture(bool);

pub fn yield_now() -> YieldFuture {
    YieldFuture(true)
}

impl Future for YieldFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            self.get_mut().0 = false;
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}
