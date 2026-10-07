//! The async runtime a generated program runs on.

use std::future::Future;

/// Builds the multi-threaded runtime, blocks on `future`, and returns its
/// output. If the runtime cannot be built, prints the reason to stderr and
/// exits with code 1.
pub fn run<F: Future>(future: F) -> F::Output {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("could not start the async runtime: {err}");
            std::process::exit(1);
        }
    };
    runtime.block_on(future)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn runs_the_future() {
        let flag = Arc::new(AtomicBool::new(false));
        let seen = flag.clone();
        super::run(async move {
            seen.store(true, Ordering::SeqCst);
        });
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn returns_the_future_s_output() {
        assert_eq!(super::run(async { 7 }), 7);
    }

    #[test]
    fn a_panic_in_the_future_propagates() {
        let result = crate::quietly(|| {
            std::panic::catch_unwind(|| {
                super::run(async {
                    panic!("boom");
                });
            })
        });
        assert!(result.is_err());
    }
}
