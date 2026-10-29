//! Interruption support. SIGINT sets a flag; blocking HTTP work runs on a helper
//! thread so the command can stop promptly and classify the interrupted operation.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

static INTERRUPTED: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// Installs the SIGINT flag once. Failure to install is non-fatal.
pub fn install() {
    INTERRUPTED.get_or_init(|| {
        let flag = Arc::new(AtomicBool::new(false));
        let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, flag.clone());
        flag
    });
}

pub fn is_interrupted() -> bool {
    INTERRUPTED
        .get()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
}

/// Why blocking work stopped before completing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Interrupted,
    TimedOut,
    Failed,
}

/// Runs blocking work on a helper thread until it finishes, the deadline passes, or
/// SIGINT arrives. The helper may continue until its own socket timeouts expire; its
/// result is then discarded.
pub fn run_until<T: Send + 'static>(
    until: Instant,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Stop> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(work());
    });
    loop {
        let now = Instant::now();
        if now >= until {
            return match receiver.try_recv() {
                Ok(value) => Ok(value),
                Err(_) => Err(Stop::TimedOut),
            };
        }
        match receiver.recv_timeout((until - now).min(Duration::from_millis(20))) {
            Ok(value) => return Ok(value),
            Err(RecvTimeoutError::Timeout) => {
                if is_interrupted() {
                    return Err(Stop::Interrupted);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Err(Stop::Failed),
        }
    }
}

/// Sleeps for `duration` unless interrupted.
pub fn sleep(duration: Duration) -> Result<(), Stop> {
    let end = Instant::now() + duration;
    loop {
        if is_interrupted() {
            return Err(Stop::Interrupted);
        }
        let now = Instant::now();
        if now >= end {
            return Ok(());
        }
        thread::sleep((end - now).min(Duration::from_millis(20)));
    }
}
