mod client;
pub mod json_rpc;
mod nip04_jsonrpc;
pub mod nip46;
mod server;
mod uds_req_res;

pub use client::*;
pub use server::*;
pub use uds_req_res::client::UdsClientError;

// TODO: Test that the client and server can communicate with each other.
