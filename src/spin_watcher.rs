use std::time::Duration;

pub(crate) async fn poll_wait<P>(predicate: P, tick_gap: u64, max_ticks: u64) -> bool
where
    P: Fn() -> bool,
{
    let mut ticks = 0;
    while !predicate() {
        if max_ticks > 0 && ticks >= max_ticks {
            return false;
        }

        tokio::time::sleep(Duration::from_millis(tick_gap)).await;
        ticks += 1;
    }

    true
}
