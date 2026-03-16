use std::time::{SystemTime, UNIX_EPOCH};

#[inline]
pub(crate) fn get_now_ms() -> u64 {
    let now = SystemTime::now();
    let duration = now.duration_since(UNIX_EPOCH).unwrap();
    duration.as_millis() as u64
}
