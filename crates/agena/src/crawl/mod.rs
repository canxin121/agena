mod plugin;

pub use plugin::{CRAWL_PLUGIN_ID, CrawlPlugin};

pub fn new_crawl_plugin(config: crate::config::CrawlConfig) -> impl crate::plugin::sdk::Plugin {
    CrawlPlugin::new(config)
}

pub fn crawl_plugin_id() -> &'static str {
    CRAWL_PLUGIN_ID
}
