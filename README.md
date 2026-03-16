# tokio-kcp-neo

`tokio-kcp-neo` 是一个基于 Tokio 的 KCP Rust library。

这个仓库内置了一份经过修改的 KCP C 实现，不依赖系统预装的 KCP 库；构建时会通过 `build.rs` 自动把 `native/src/ikcp.c` 编译成静态库，并和 Rust 代码一起链接。这份 C 代码来自 [czf0613/kcp](https://github.com/czf0613/kcp) 项目。

## 当前状态

- 提供一个 Tokio 驱动的 KCP 封装 `TokioKcp`
- 内置异步发送回调、定时 `ikcp_update` 驱动和接收缓冲区
- 已包含一个在丢包场景下验证双向传输的测试
- 当前底层 KCP C 代码是项目内维护版本，不是原版上游源码

## 构建要求

- Rust 1.85+
- 可用的 C 编译器
  - macOS: `clang`
  - Linux: `gcc` 或 `clang`
  - Windows: 需要可用的 MSVC 或 MinGW 工具链

## 安装

当前推荐直接通过 git URL 引用这个仓库，并指定对应版本 tag，不需要先克隆到本地：

```toml
[dependencies]
tokio-kcp-neo = { git = "https://github.com/czf0613/TokioKcp.git", tag = "0.0.1" }
```

## 设计说明

`TokioKcp` 的工作方式比较直接：

1. `write()` 把数据交给 KCP
2. KCP 需要发包时，通过你提供的异步回调把 datagram 发到底层传输层
3. 你从 UDP 或其他不可靠传输层收到 datagram 后，调用 `enqueue()` 喂回 KCP
4. KCP 组包完成后，通过 `read()` 按指定长度读出数据

## API

当前公开的核心接口如下：

```rust
impl TokioKcp {
    pub const DEFAULT_MTU: u32 = 1400;

    pub fn new(conv_id: u32, on_send: DGCallBack) -> Self;
    pub fn with_mtu(conv_id: u32, mtu: u32, on_send: DGCallBack) -> Self;

    pub fn write(&self, data: &[u8]);
    pub fn enqueue(&self, data: &[u8]);
    pub async fn read(&self, exact_bytes: usize) -> Vec<u8>;
    pub async fn shutdown(self);
}
```

发送回调类型：

```rust
pub type DGCallBack =
    for<'a> fn(&'a [u8]) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;
```

它的含义是：当 KCP 需要把底层报文发出去时，你需要提供一个异步函数，把这段字节真正发到 UDP socket、隧道或你自己的传输层里。

## 最小使用流程

```rust
use tokio_kcp_neo::TokioKcp;

// 1. 定义底层发送回调
// 2. 创建 TokioKcp::new(conv, callback)
// 3. 上层写数据时调用 write()
// 4. 底层收到 datagram 时调用 enqueue()
// 5. 业务侧通过 read(exact_bytes).await 取回完整数据
// 6. 结束时调用 shutdown().await
```

更完整的可运行用法可以参考测试文件：

- [src/test.rs](./src/test.rs)

## 测试

```bash
cargo test
```

当前测试会构造两个 KCP 端点，并在 50% 随机丢包条件下验证双向 payload 仍然能够完整传输。

## 项目结构

```text
.
├── build.rs
├── native
│   ├── include/ikcp.h
│   └── src/ikcp.c
└── src
    ├── lib.rs
    ├── native_code.rs
    ├── spin_watcher.rs
    ├── test.rs
    └── time_utils.rs
```

## 说明

- `read(exact_bytes)` 目前是按“精确长度”读取的接口
- 当前 API 还比较轻量，后续可以继续扩展成更完整的 stream 风格封装
- C 构建产物是 Cargo 的中间产物，通常位于 `target/<profile>/build/<pkg-hash>/out/`
