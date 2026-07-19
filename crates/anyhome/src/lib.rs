//! anyhome: a mountable NFSv3 memory service.
//!
//! Splits NFS traffic between a real filesystem layer (enumerable memory stored
//! as files/dirs) and a DB layer (vector search exposed via magic paths like
//! `/search/<query>` and `/index/<key>`).

pub mod backend;
pub mod cli;
pub mod db;
pub mod dispatcher;
pub mod magic;

pub use backend::RealFs;
pub use cli::Cli;
pub use db::{embedder, MemoryDb};
pub use dispatcher::AnyHomeFs;
