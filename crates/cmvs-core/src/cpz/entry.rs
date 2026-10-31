//! Public archive metadata; cipher keys belong to each version implementation.
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Entry {
    pub directory: Arc<[u8]>,
    pub name: Vec<u8>,
    pub size: u32,
    pub offset: u64,
    pub checksum_raw: u32,
}
