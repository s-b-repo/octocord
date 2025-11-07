use once_cell::sync::Lazy;
use tokio::runtime::{Builder, Handle, Runtime};

static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("octocord-rt")
        .build()
        .expect("Failed to build Tokio runtime")
});

pub fn runtime_handle() -> Handle {
    RUNTIME.handle().clone()
}


