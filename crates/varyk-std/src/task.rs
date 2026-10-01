//! `Task<T>`: a started async call.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::future::join_all;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::task::JoinHandle;

/// A call running on the runtime. Dropping it cancels the work; awaiting it
/// gives the result; `detach` lets it run on unwatched.
pub struct Task<T> {
    handle: Option<JoinHandle<T>>,
}

impl<T> Task<T> {
    /// Starts `future` on the runtime.
    pub fn start<F>(future: F) -> Task<F::Output>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        Task {
            handle: Some(tokio::spawn(future)),
        }
    }

    /// Lets the task run on with nobody waiting for it.
    pub fn detach(mut self) {
        // Dropping a JoinHandle detaches its task; `Task::drop` then has
        // nothing left to abort.
        self.handle = None;
    }

    /// Waits for every task and gives the results in the list's order.
    pub async fn all(tasks: Vec<Task<T>>) -> Vec<T> {
        join_all(tasks).await
    }
}

impl<U, E> Task<Result<U, E>> {
    /// Like `all` for `Result` tasks: the `Ok` values in order, or the first
    /// `Err` to arrive, cancelling the tasks still running.
    pub async fn try_all(tasks: Vec<Task<Result<U, E>>>) -> Result<Vec<U>, E> {
        let mut pending = FuturesUnordered::new();
        for (index, task) in tasks.into_iter().enumerate() {
            pending.push(async move { (index, task.await) });
        }
        let mut oks = Vec::with_capacity(pending.len());
        while let Some((index, outcome)) = pending.next().await {
            match outcome {
                Ok(value) => oks.push((index, value)),
                Err(err) => return Err(err),
            }
        }
        oks.sort_by_key(|(index, _)| *index);
        Ok(oks.into_iter().map(|(_, value)| value).collect())
    }
}

impl<T> Future for Task<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let Some(handle) = self.handle.as_mut() else {
            return Poll::Pending;
        };
        match Pin::new(handle).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(value)) => Poll::Ready(value),
            Poll::Ready(Err(err)) => match err.try_into_panic() {
                Ok(payload) => std::panic::resume_unwind(payload),
                // Cancelled under its waiter: only while the runtime shuts
                // down, so leave the waiter pending.
                Err(_) => Poll::Pending,
            },
        }
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Task;
    use crate::time::sleep;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn awaiting_gives_the_value() {
        let got = Arc::new(AtomicUsize::new(0));
        let out = got.clone();
        crate::run(async move {
            let t = Task::start(async { 7usize });
            out.store(t.await, Ordering::SeqCst);
        });
        assert_eq!(got.load(Ordering::SeqCst), 7);
    }

    #[test]
    fn dropping_an_unfinished_task_aborts_it() {
        let set = flag();
        let seen = set.clone();
        crate::run(async move {
            let t = Task::start(async move {
                sleep(50).await;
                seen.store(true, Ordering::SeqCst);
            });
            sleep(5).await;
            drop(t);
            sleep(120).await;
        });
        assert!(!set.load(Ordering::SeqCst));
    }

    #[test]
    fn detach_keeps_it_running() {
        let set = flag();
        let seen = set.clone();
        crate::run(async move {
            Task::start(async move {
                sleep(20).await;
                seen.store(true, Ordering::SeqCst);
            })
            .detach();
            sleep(100).await;
        });
        assert!(set.load(Ordering::SeqCst));
    }

    #[test]
    fn a_panic_resumes_in_the_waiter() {
        let result = crate::quietly(|| {
            std::panic::catch_unwind(|| {
                crate::run(async {
                    Task::start(async {
                        panic!("task boom");
                    })
                    .await
                });
            })
        });
        assert!(result.is_err());
    }

    #[test]
    fn an_aborted_task_leaves_the_waiter_pending() {
        use std::future::Future;
        use std::pin::Pin;
        use std::task::{Context, Poll, Waker};
        let pending = flag();
        let out = pending.clone();
        crate::run(async move {
            let mut t = Task::start(async {
                sleep(10_000).await;
            });
            if let Some(handle) = t.handle.as_ref() {
                handle.abort();
            }
            sleep(20).await;
            let mut cx = Context::from_waker(Waker::noop());
            out.store(
                matches!(Pin::new(&mut t).poll(&mut cx), Poll::Pending),
                Ordering::SeqCst,
            );
        });
        assert!(pending.load(Ordering::SeqCst));
    }

    #[test]
    fn all_keeps_order_when_tasks_finish_out_of_order() {
        let got = Arc::new(std::sync::Mutex::new(Vec::new()));
        let out = got.clone();
        crate::run(async move {
            let tasks = vec![
                Task::start(async {
                    sleep(40).await;
                    1
                }),
                Task::start(async {
                    sleep(20).await;
                    2
                }),
                Task::start(async { 3 }),
            ];
            let values = Task::all(tasks).await;
            if let Ok(mut g) = out.lock() {
                *g = values;
            }
        });
        assert_eq!(got.lock().map(|g| g.clone()).unwrap_or_default(), [1, 2, 3]);
    }

    #[test]
    fn all_of_nothing_is_empty() {
        crate::run(async {
            let none: Vec<Task<u8>> = Vec::new();
            assert!(Task::all(none).await.is_empty());
        });
    }

    type Outcome = Result<Vec<usize>, String>;

    fn run_try_all(
        make: impl FnOnce() -> Vec<Task<Result<usize, String>>> + Send + 'static,
    ) -> Outcome {
        let slot = Arc::new(std::sync::Mutex::new(None));
        let out = slot.clone();
        crate::run(async move {
            let r = Task::try_all(make()).await;
            if let Ok(mut g) = out.lock() {
                *g = Some(r);
            }
        });
        slot.lock()
            .ok()
            .and_then(|mut g| g.take())
            .unwrap_or_else(|| Err("no result".to_string()))
    }

    #[test]
    fn try_all_keeps_order_of_oks() {
        let got = run_try_all(|| {
            (0..5usize)
                .map(|i| {
                    Task::start(async move {
                        sleep((5 - i as u64) * 10).await;
                        Ok(i)
                    })
                })
                .collect()
        });
        assert_eq!(got, Ok(vec![0, 1, 2, 3, 4]));
    }

    #[test]
    fn try_all_of_nothing_is_ok_empty() {
        assert_eq!(run_try_all(Vec::new), Ok(vec![]));
    }

    #[test]
    fn try_all_returns_the_first_err_with_forty_tasks_and_aborts_the_rest() {
        let finished = Arc::new(AtomicUsize::new(0));
        let counter = finished.clone();
        let result = Arc::new(std::sync::Mutex::new(None));
        let out = result.clone();
        // One runtime throughout: the rest must be cancelled by try_all
        // itself, not by the runtime shutting down.
        crate::run(async move {
            let tasks: Vec<Task<Result<usize, String>>> = (0..40usize)
                .map(|i| {
                    let counter = counter.clone();
                    Task::start(async move {
                        if i == 39 {
                            return Err("last failed first".to_string());
                        }
                        sleep(100).await;
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(i)
                    })
                })
                .collect();
            let r = Task::try_all(tasks).await;
            // Long enough for any task left running to finish.
            sleep(300).await;
            if let Ok(mut g) = out.lock() {
                *g = Some(r);
            }
        });
        let got = result.lock().ok().and_then(|mut g| g.take());
        assert_eq!(got, Some(Err("last failed first".to_string())));
        assert_eq!(finished.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_panicking_task_panics_the_waiter_in_all_and_try_all() {
        let all = crate::quietly(|| {
            std::panic::catch_unwind(|| {
                crate::run(async {
                    let tasks = vec![
                        Task::start(async {
                            sleep(10).await;
                            1
                        }),
                        Task::start(async { panic!("boom") }),
                    ];
                    Task::all(tasks).await;
                });
            })
        });
        assert!(all.is_err());
        let try_all = crate::quietly(|| {
            std::panic::catch_unwind(|| {
                run_try_all(|| {
                    vec![
                        Task::start(async {
                            sleep(10).await;
                            Ok(1)
                        }),
                        Task::start(async { panic!("boom") }),
                    ]
                })
            })
        });
        assert!(try_all.is_err());
    }
}
