//! Separate admission limits for synchronous runtime service dependencies.

use agena_async::BlockingPool;

pub(crate) static FILE_OPERATIONS: BlockingPool = BlockingPool::new(8);
pub(crate) static PLUGIN_STORAGE: BlockingPool = BlockingPool::new(8);
pub(crate) static PLUGIN_SECRETS: BlockingPool = BlockingPool::new(4);
pub(crate) static IMAGE_PROCESSING: BlockingPool = BlockingPool::new(2);
pub(crate) static PROCESS_LOG_READS: BlockingPool = BlockingPool::new(16);
