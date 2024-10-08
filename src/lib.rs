#![warn(clippy::all, clippy::nursery, clippy::pedantic)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

mod client;
pub mod json_rpc;
mod nip04_jsonrpc;
pub mod nip_46;
mod server;
mod stream_helper;
mod uds_req_res;

pub use client::*;
pub use server::*;
pub use uds_req_res::client::UdsClientError;

// TODO: Test that the client and server can communicate with each other.
