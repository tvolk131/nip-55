mod client;
pub mod json_rpc;
mod nip04_jsonrpc;
mod server;
mod uds_req_res;

pub use client::*;
// pub use json_rpc::{JsonRpcRequest, JsonRpcResponse};
pub use server::*;

// TODO: Test that the client and server can communicate with each other.
