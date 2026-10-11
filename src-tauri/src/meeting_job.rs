//! Scoped meeting cancellation. A late CLI disconnect cannot stop a newer job.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
pub const STOPPED: &str = "Meeting processing stopped";
static ACTIVE: Mutex<Option<(String, Arc<AtomicBool>)>> = Mutex::new(None);
pub struct MeetingJob {
    pub id: String,
    pub cancel: Arc<AtomicBool>,
}
impl MeetingJob {
    pub fn begin(id: Option<String>) -> Result<Self, String> {
        let mut active = ACTIVE.lock().map_err(|e| e.to_string())?;
        if active.is_some() {
            return Err("Another meeting is still processing".into());
        }
        let job = Self {
            id: id.unwrap_or_else(|| format!("meeting-{:032x}", rand::random::<u128>())),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        *active = Some((job.id.clone(), job.cancel.clone()));
        Ok(job)
    }
}
impl Drop for MeetingJob {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE.lock() {
            if active
                .as_ref()
                .is_some_and(|(_, flag)| Arc::ptr_eq(flag, &self.cancel))
            {
                *active = None;
            }
        }
    }
}
fn cancel_owner(active: &Option<(String, Arc<AtomicBool>)>, id: Option<&str>) -> bool {
    if let Some((owner, flag)) = active {
        if id.is_none_or(|id| id == owner) {
            flag.store(true, Ordering::Relaxed);
            return true;
        }
    }
    false
}
pub fn cancel(id: Option<&str>) -> Result<bool, String> {
    let active = ACTIVE.lock().map_err(|e| e.to_string())?;
    Ok(cancel_owner(&active, id))
}
/// Atomically close the cancellation window before committing saved results.
pub fn finish(flag: &Arc<AtomicBool>) -> Result<(), String> {
    let mut active = ACTIVE.lock().map_err(|e| e.to_string())?;
    check(flag)?;
    if active
        .as_ref()
        .is_some_and(|(_, current)| Arc::ptr_eq(current, flag))
    {
        *active = None;
    }
    Ok(())
}
pub fn current_cancel() -> Option<Arc<AtomicBool>> {
    ACTIVE
        .lock()
        .ok()
        .and_then(|active| active.as_ref().map(|(_, flag)| flag.clone()))
}
pub fn check(flag: &AtomicBool) -> Result<(), String> {
    if flag.load(Ordering::Relaxed) {
        Err(STOPPED.into())
    } else {
        Ok(())
    }
}
pub fn check_current() -> Result<(), String> {
    current_cancel().map_or(Ok(()), |flag| check(&flag))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_disconnect_cannot_cancel_next_job() {
        let old = Arc::new(AtomicBool::new(false));
        let active = Some(("old".into(), old.clone()));
        assert!(cancel_owner(&active, Some("old")));
        assert!(check(&old).is_err());
        let next = Arc::new(AtomicBool::new(false));
        let active = Some(("next".into(), next.clone()));
        assert!(!cancel_owner(&active, Some("old")));
        assert!(check(&next).is_ok());
        assert!(cancel_owner(&active, None));
    }
}

/// Bridge the app flag to the native runtime, with a watcher that always joins.
pub struct NativeCancelWatch {
    pub token: transcribe_cpp::CancelToken,
    done: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl NativeCancelWatch {
    pub fn new(cancel: Arc<AtomicBool>) -> Self {
        let token = transcribe_cpp::CancelToken::new();
        let done = Arc::new(AtomicBool::new(false));
        let worker = {
            let token = token.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        token.cancel();
                        break;
                    }
                    std::thread::park_timeout(std::time::Duration::from_millis(20));
                }
            })
        };
        Self {
            token,
            done,
            worker: Some(worker),
        }
    }
}
impl Drop for NativeCancelWatch {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
