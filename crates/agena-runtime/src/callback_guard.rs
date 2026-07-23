use std::sync::Mutex;

/// Runs a one-shot callback when the guard is dropped.
pub struct CallbackOnDrop {
    callback: Mutex<Option<Box<dyn FnOnce() + Send + 'static>>>,
}

impl CallbackOnDrop {
    pub fn new(callback: impl FnOnce() + Send + 'static) -> Self {
        Self {
            callback: Mutex::new(Some(Box::new(callback))),
        }
    }
}

impl Drop for CallbackOnDrop {
    fn drop(&mut self) {
        if let Some(callback) = self
            .callback
            .lock()
            .expect("callback guard lock poisoned")
            .take()
        {
            callback();
        }
    }
}
