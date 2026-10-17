//! The browser build compiled into the app (`FIGCRAFT_WEB_DIST` at build time; see `build.rs`):
//! sessions the app hosts serve it on their port, so a phone on the network opens the address
//! Share ▸ View on another device gives and watches the file. A build without it has no site.

use figcraft_collab_net::site::{Site, SiteFile, content_type};

mod bundled {
    include!(concat!(env!("OUT_DIR"), "/web_site.rs"));
}

/// The compiled-in site, if this build has one.
pub fn site() -> Option<Site> {
    let files: Vec<SiteFile> = bundled::FILES.iter().map(|(path, gzip)| SiteFile { path, content_type: content_type(path), gzip }).collect();
    let site = Site::new(files);
    site.has_page().then_some(site)
}
