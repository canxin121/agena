#[cfg(target_endian = "big")]
#[path = "index_portable.rs"]
mod backend;
#[cfg(target_endian = "little")]
#[path = "index_tantivy.rs"]
mod backend;

pub use backend::{rebuild_search_index, search_documents};
