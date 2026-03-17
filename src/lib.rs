//! A Tokio-based KCP library for Rust.

mod native_code;
mod spin_watcher;
mod time_utils;

#[cfg(test)]
mod test;

use native_code::{
    KcpHandle, ikcp_create, ikcp_input, ikcp_recv, ikcp_send, ikcp_setmtu, ikcp_setoutput,
    ikcp_update, ikcpcb,
};
use spin_watcher::poll_wait;
use std::{
    cmp::max,
    collections::VecDeque,
    ffi::{c_int, c_long, c_void},
    future::Future,
    pin::Pin,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use time_utils::get_now_ms;
use tokio::{
    sync::{Mutex, Notify, mpsc},
    task,
};

enum KcpAction {
    Update,
    DGSocket(Vec<u8>),
    Write(Vec<u8>),
    Enqueue(Vec<u8>),
}

pub type DGCallBack = for<'a> fn(&'a [u8]) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

struct KcpCtx {
    dispatch_queue: mpsc::UnboundedSender<KcpAction>,
}

extern "C" fn kcp_cb_cfn(
    buf: *const u8,
    len: c_int,
    _kcp: *mut ikcpcb,
    user: *mut c_void,
) -> c_int {
    let ctx = unsafe { &*(user as *const KcpCtx) };
    let payload = unsafe { std::slice::from_raw_parts(buf, len as usize) }.to_vec();

    match ctx.dispatch_queue.send(KcpAction::DGSocket(payload)) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

pub struct TokioKcp {
    kcp_obj: Arc<KcpHandle>,
    kcp_ctx: Box<KcpCtx>,
    stop_flag: Arc<AtomicBool>,
    time_driver: task::JoinHandle<()>,
    loop_driver: task::JoinHandle<()>,
    read_buffer: Arc<Mutex<VecDeque<u8>>>,
    read_notify: Arc<Notify>,
}

static TIME_BASE: LazyLock<u64> = LazyLock::new(get_now_ms);

#[inline]
fn get_offset_time_ms() -> u32 {
    let base = *TIME_BASE;
    let now = get_now_ms();
    max(now - base, 1) as u32
}

impl TokioKcp {
    pub const DEFAULT_MTU: u32 = 1400;
    pub const DEFAULT_REFRESH_GAP: u64 = 20;

    pub fn new(conv_id: u32, on_send: DGCallBack) -> Self {
        Self::with_mtu_and_refresh_gap(
            conv_id,
            Self::DEFAULT_MTU,
            Self::DEFAULT_REFRESH_GAP,
            on_send,
        )
    }

    pub fn with_mtu(conv_id: u32, mtu: u32, on_send: DGCallBack) -> Self {
        Self::with_mtu_and_refresh_gap(conv_id, mtu, Self::DEFAULT_REFRESH_GAP, on_send)
    }

    pub fn with_mtu_and_refresh_gap(
        conv_id: u32,
        mtu: u32,
        refresh_gap: u64,
        on_send: DGCallBack,
    ) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<KcpAction>();
        let transport = on_send;

        let mut ctx_box = Box::new(KcpCtx {
            dispatch_queue: tx.clone(),
        });

        let ctx_ptr = (&mut *ctx_box) as *mut KcpCtx;
        let raw = ikcp_create(conv_id, ctx_ptr.cast::<c_void>());
        assert!(!raw.is_null(), "ikcp_create returned a null pointer");

        let cb = Arc::new(KcpHandle(raw));
        let cb1 = cb.clone();
        ikcp_setoutput(cb.0, kcp_cb_cfn);
        ikcp_setmtu(cb.0, mtu as c_int);

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag1 = stop_flag.clone();
        let stop_flag2 = stop_flag.clone();

        let read_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(1024 * 1024)));
        let read_buffer1 = read_buffer.clone();
        let read_notify = Arc::new(Notify::new());
        let read_notify1 = read_notify.clone();

        Self {
            kcp_obj: cb,
            kcp_ctx: ctx_box,
            stop_flag,

            time_driver: tokio::spawn(async move {
                while !stop_flag1.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(refresh_gap)).await;
                    let _ = tx.send(KcpAction::Update);
                }
            }),

            loop_driver: tokio::spawn(async move {
                let mut temp_buffer = vec![0u8; 1024 * 1024];

                loop {
                    tokio::select! {
                        Some(action) = rx.recv() => {
                            match action {
                                KcpAction::Update => {
                                    ikcp_update(cb1.0, get_offset_time_ms());

                                    loop {
                                        let bytes_read = ikcp_recv(
                                            cb1.0,
                                            temp_buffer.as_mut_ptr(),
                                            temp_buffer.len() as c_int,
                                        );

                                        if bytes_read <= 0 {
                                            break;
                                        }

                                        {
                                            let mut buffer_lock = read_buffer1.lock().await;
                                            buffer_lock.extend(temp_buffer[..bytes_read as usize].iter());
                                        }

                                        read_notify1.notify_waiters();
                                    }
                                }
                                KcpAction::DGSocket(data) => {
                                    if !data.is_empty() {
                                        let _ = (transport)(&data).await;
                                    }
                                }
                                KcpAction::Write(data) => {
                                    if !data.is_empty() {
                                        ikcp_send(cb1.0, data.as_ptr(), data.len() as c_int);
                                    }
                                }
                                KcpAction::Enqueue(data) => {
                                    if !data.is_empty() {
                                        ikcp_input(cb1.0, data.as_ptr(), data.len() as c_long);
                                    }
                                }
                            }
                        }
                        _ = poll_wait(|| stop_flag2.load(Ordering::Acquire), 100, 0) => {
                            break;
                        }
                    }
                }
            }),

            read_buffer,
            read_notify,
        }
    }

    pub fn write(&self, data: &[u8]) {
        let _ = self
            .kcp_ctx
            .dispatch_queue
            .send(KcpAction::Write(data.to_vec()));
    }

    pub async fn read(&self, exact_bytes: usize) -> Vec<u8> {
        if exact_bytes == 0 {
            return Vec::new();
        }

        loop {
            let notified = self.read_notify.notified();
            let mut buffer_lock = self.read_buffer.lock().await;
            if buffer_lock.len() >= exact_bytes {
                return buffer_lock.drain(..exact_bytes).collect();
            }

            drop(buffer_lock);
            notified.await;
        }
    }

    pub fn enqueue(&self, data: &[u8]) {
        let _ = self
            .kcp_ctx
            .dispatch_queue
            .send(KcpAction::Enqueue(data.to_vec()));
    }

    pub async fn shutdown(self) {
        self.stop_flag.store(true, Ordering::Release);
        tokio::time::sleep(Duration::from_millis(100)).await;

        self.time_driver.abort();
        self.loop_driver.abort();
        let _ = self.time_driver.await;
        let _ = self.loop_driver.await;

        drop(self.read_notify);
        drop(self.read_buffer);
        drop(self.kcp_ctx);
        drop(self.kcp_obj);
    }
}
