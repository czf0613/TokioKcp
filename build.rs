fn main() {
    cc::Build::new()
        .file("native/src/ikcp.c")
        .include("native/include")
        .compile("kcp");
}
