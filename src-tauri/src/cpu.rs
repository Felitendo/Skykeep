//! Where the app's heavy picture work runs, by who it is for.
//!
//! For the screen (a tile or a photo the window has asked for and is waiting on): on the
//! runtime's blocking threads, every core if need be. For later (pictures made ahead, a HEIC
//! decoded before it is opened): on one worker thread at the lowest CPU priority, so it only
//! ever has the time the window is not using. Made ahead on every core at normal priority, the
//! work starved the window — his scrolling froze for most of a second at a time while a newly
//! synced library was being prepared.

use std::sync::mpsc;
use std::sync::OnceLock;

tokio::task_local! {
    /// Set for work done ahead of being asked for (see `ahead`).
    static AHEAD: bool;
}

/// Runs `work` as work done ahead: everything it does on the CPU (through `run`) goes to the
/// background worker.
pub async fn ahead<F: std::future::Future>(work: F) -> F::Output {
    AHEAD.scope(true, work).await
}

/// Whether the current task is work done ahead.
pub fn is_ahead() -> bool {
    AHEAD.try_with(|a| *a).unwrap_or(false)
}

type Job = Box<dyn FnOnce() + Send>;

/// The one background worker, started at the lowest priority the first time it is needed.
fn background() -> &'static mpsc::Sender<Job> {
    static TX: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("quince-ahead".into())
            .spawn(move || {
                // Nice 19 for this thread alone (Linux schedules threads, so the window's own
                // threads keep their place).
                #[cfg(target_os = "linux")]
                unsafe {
                    let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
                    libc::setpriority(libc::PRIO_PROCESS, tid, 19);
                }
                for job in rx {
                    job();
                }
            })
            .expect("could not start the background worker");
        tx
    })
}

/// Runs CPU-heavy `work` where it belongs: on the background worker inside `ahead`, otherwise
/// on the runtime's blocking threads.
pub async fn run<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if is_ahead() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        background()
            .send(Box::new(move || {
                let _ = tx.send(work());
            }))
            .map_err(|_| "the background worker has stopped".to_string())?;
        rx.await
            .map_err(|_| "the background worker dropped the work".to_string())
    } else {
        tokio::task::spawn_blocking(work)
            .await
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn work_ahead_runs_on_the_low_priority_worker_and_the_rest_does_not() {
        tauri::async_runtime::block_on(async {
            let name = || std::thread::current().name().map(str::to_string);
            let now = super::run(name).await.unwrap();
            let later = super::ahead(super::run(name)).await.unwrap();
            assert_ne!(now.as_deref(), Some("quince-ahead"));
            assert_eq!(later.as_deref(), Some("quince-ahead"));
            #[cfg(target_os = "linux")]
            {
                let nice = super::ahead(super::run(|| unsafe {
                    libc::getpriority(
                        libc::PRIO_PROCESS,
                        libc::syscall(libc::SYS_gettid) as libc::id_t,
                    )
                }))
                .await
                .unwrap();
                assert_eq!(nice, 19);
            }
        });
    }
}
