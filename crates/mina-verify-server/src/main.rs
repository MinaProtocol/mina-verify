//! Thin entry point — all logic lives in the library (`mina_verify_server`) so it can be
//! unit- and integration-tested without spawning a process. See the crate docs there.

fn main() {
    mina_verify_server::run();
}
