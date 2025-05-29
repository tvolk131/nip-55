#![warn(clippy::all, clippy::nursery, clippy::pedantic)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

//! # NIP-55 Reference Implementation
//!
//! This crate provides a reference implementation of Nostr NIP-55, supporting both
//! traditional request-response patterns and pipelined requests over Unix domain sockets.
//!
//! ## Features
//!
//! - **Traditional Transport**: One request per connection (`UnixDomainSocketClientTransport`)
//! - **Pipelined Transport**: Multiple concurrent requests over persistent connections
//! - **JSON-RPC 2.0**: Full support for JSON-RPC requests and responses
//! - **NIP-46 Integration**: Seamless integration with Nostr's remote signing protocol
//!
//! ## Pipelined Transport
//!
//! The pipelined transport implements [Option 3](https://www.simple-is-better.org/json-rpc/transport_sockets.html)
//! from the JSON-RPC transport specification, allowing multiple requests to be sent
//! without waiting for individual responses. See [`pipelined_client`] and [`pipelined_server`]
//! modules for more details.

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
pub use uds_req_res::pipelined_client;
pub use uds_req_res::pipelined_server;

// TODO: Test that the client and server can communicate with each other.
