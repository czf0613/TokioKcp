use crate::TokioKcp;
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock, Mutex, Weak},
    time::Duration,
};

type TransportFuture = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

static LEFT_PEER: LazyLock<Mutex<Option<Weak<TokioKcp>>>> = LazyLock::new(|| Mutex::new(None));
static RIGHT_PEER: LazyLock<Mutex<Option<Weak<TokioKcp>>>> = LazyLock::new(|| Mutex::new(None));
static DROP_RNG: LazyLock<Mutex<StdRng>> =
    LazyLock::new(|| Mutex::new(StdRng::seed_from_u64(0x5eed_baad_f00d_u64)));

fn should_drop() -> bool {
    DROP_RNG.lock().unwrap().random_bool(0.3)
}

fn peer_transport(
    peer_slot: &'static LazyLock<Mutex<Option<Weak<TokioKcp>>>>,
) -> impl Fn(Vec<u8>) -> TransportFuture + Copy {
    move |payload: Vec<u8>| {
        let peer = peer_slot.lock().unwrap().clone();
        let drop_packet = should_drop();

        Box::pin(async move {
            if drop_packet {
                return true;
            }

            let Some(peer) = peer.and_then(|it| it.upgrade()) else {
                return false;
            };

            peer.enqueue(&payload);
            true
        })
    }
}

fn make_payload(len: usize, salt: u8) -> Vec<u8> {
    (0..len)
        .map(|idx| ((idx as u32 * 31 + salt as u32) % 251) as u8)
        .collect()
}

fn unwrap_arc<T>(value: Arc<T>, name: &str) -> T {
    match Arc::try_unwrap(value) {
        Ok(inner) => inner,
        Err(_) => panic!("{name} still has outstanding strong references"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn tokiokcp_can_transfer_complete_payload_under_packet_loss() {
    let left = Arc::new(TokioKcp::new(7, peer_transport(&RIGHT_PEER)));
    let right = Arc::new(TokioKcp::new(7, peer_transport(&LEFT_PEER)));

    *LEFT_PEER.lock().unwrap() = Some(Arc::downgrade(&left));
    *RIGHT_PEER.lock().unwrap() = Some(Arc::downgrade(&right));

    let left_payload = make_payload(16 * 1024, 7);
    let right_payload = make_payload(12 * 1024, 19);

    left.write(&left_payload);
    right.write(&right_payload);

    let right_read =
        tokio::time::timeout(Duration::from_secs(60), right.read_exact(left_payload.len()));
    let left_read =
        tokio::time::timeout(Duration::from_secs(60), left.read_exact(right_payload.len()));

    let (right_result, left_result) = tokio::join!(right_read, left_read);

    let received_by_right = right_result.expect("right side timed out while receiving");
    let received_by_left = left_result.expect("left side timed out while receiving");

    assert_eq!(received_by_right, left_payload);
    assert_eq!(received_by_left, right_payload);

    *LEFT_PEER.lock().unwrap() = None;
    *RIGHT_PEER.lock().unwrap() = None;

    unwrap_arc(left, "left").shutdown().await;
    unwrap_arc(right, "right").shutdown().await;
}
