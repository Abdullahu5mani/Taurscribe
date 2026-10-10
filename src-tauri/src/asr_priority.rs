//! File inference yields to capture and its final pass at a chunk boundary.
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

#[derive(Default)]
struct Activity {
    foreground: usize,
    background: bool,
}
#[derive(Default)]
pub(crate) struct Priority {
    activity: Mutex<Activity>,
    changed: Condvar,
}
static PRIORITY: OnceLock<Arc<Priority>> = OnceLock::new();
pub(crate) fn shared() -> &'static Arc<Priority> {
    PRIORITY.get_or_init(|| Arc::new(Priority::default()))
}

pub(crate) struct Foreground {
    priority: Arc<Priority>,
}
pub(crate) struct Background {
    priority: Arc<Priority>,
}
impl Priority {
    /// Reserve before waiting, preventing the file from taking its next chunk.
    pub(crate) fn foreground(self: &Arc<Self>) -> Foreground {
        self.activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .foreground += 1;
        Foreground {
            priority: self.clone(),
        }
    }
    pub(crate) fn background(
        self: &Arc<Self>,
        mut while_waiting: impl FnMut() -> Result<(), String>,
    ) -> Result<Background, String> {
        let mut activity = self.activity.lock().unwrap_or_else(|e| e.into_inner());
        while activity.foreground > 0 || activity.background {
            drop(activity);
            while_waiting()?;
            activity = self.activity.lock().unwrap_or_else(|e| e.into_inner());
            if activity.foreground > 0 || activity.background {
                activity = self
                    .changed
                    .wait_timeout(activity, Duration::from_millis(50))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
        activity.background = true;
        Ok(Background {
            priority: self.clone(),
        })
    }
}
impl Foreground {
    pub(crate) fn wait_for_file_chunk(&self) {
        let mut activity = self
            .priority
            .activity
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while activity.background {
            activity = self
                .priority
                .changed
                .wait(activity)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}
impl Drop for Foreground {
    fn drop(&mut self) {
        self.priority
            .activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .foreground -= 1;
        self.priority.changed.notify_all();
    }
}
impl Drop for Background {
    fn drop(&mut self) {
        self.priority
            .activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .background = false;
        self.priority.changed.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recording_waits_for_chunk_then_keeps_file_paused_through_final_pass() {
        let priority = Arc::new(Priority::default());
        let chunk = priority.background(|| Ok(())).unwrap();
        let capture = priority.foreground();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            capture.wait_for_file_chunk();
            tx.send(capture).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(40)).is_err());
        drop(chunk);
        let final_pass = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        thread.join().unwrap();
        assert_eq!(
            priority
                .background(|| Err("cancelled while paused".into()))
                .err()
                .unwrap(),
            "cancelled while paused"
        );
        drop(final_pass);
        assert!(priority.background(|| panic!("file should resume")).is_ok());
    }
    #[test]
    fn detached_recording_worker_keeps_priority_and_releases_on_failure() {
        let priority = Arc::new(Priority::default());
        let recording = priority.foreground();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _recording = recording;
            rx.recv().unwrap();
        });
        assert!(priority.background(|| Err("paused".into())).is_err());
        tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(priority
            .background(|| panic!("failure must release priority"))
            .is_ok());
    }
}
