//! Public-safe transport primitives for the Gate 3 C facade.
//!
//! This crate currently owns only the versioned transport envelope. The
//! process host, asynchronous operations, callbacks, and exported C ABI are
//! deliberately left for G3-05. Domain structs, database rows, and runtime
//! handles must not be added to this module.

pub mod transport;

pub use transport::{
    ExtensionRequest, FfiError, FfiRequest, FfiResponse, MAX_EVENT_BYTES, MAX_MESSAGE_BYTES,
    RequestBody, ResponseBody, SCHEMA_MAJOR, TransportError,
};
