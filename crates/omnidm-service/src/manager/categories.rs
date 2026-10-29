// SPDX-License-Identifier: GPL-3.0-or-later

//! Categories: which one a download belongs to, and so where a browser download is saved.

use std::path::{Path, PathBuf};

use omnidm_ipc::protocol::CategorySummary;
use omnidm_net::cookies::Url;
use omnidm_queue::category::{FileFacts, pick};
use omnidm_store::{CategoryId, CategoryRecord};

use super::Manager;

impl Manager {
    /// Every category, in the order they are tried.
    pub fn categories(&self) -> Vec<CategorySummary> {
        let store = self.inner.store.lock().unwrap();
        store
            .list_categories()
            .unwrap_or_default()
            .into_iter()
            .map(|c| CategorySummary {
                category_id: c.id.0,
                name: c.settings.name,
                extensions: c.settings.rule.extensions,
                mime_types: c.settings.rule.mime_types,
                hosts: c.settings.rule.hosts,
                folder: c.settings.save_dir,
            })
            .collect()
    }

    /// Sets where a category's browser downloads are saved; `None` (or blank) means the
    /// downloads folder. The folder is made when a download first needs it.
    pub fn set_category_folder(&self, name: &str, folder: Option<String>) -> Result<(), String> {
        let folder = folder
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty());
        if let Some(folder) = &folder
            && !Path::new(folder).is_absolute()
        {
            return Err(format!("{folder:?} isn't a full path, like D:\\Videos"));
        }
        let mut store = self.inner.store.lock().unwrap();
        let category = find(&store.list_categories().map_err(|e| e.to_string())?, name)?;
        let mut settings = category.settings;
        settings.save_dir = folder;
        store
            .update_category(category.id, &settings)
            .map_err(|e| e.to_string())
    }

    /// The category of a file, by its name, MIME type and URL, and the folder to save it
    /// in: the category's own folder, or `fallback`. A category folder that can't be made
    /// falls back too.
    pub(super) fn place(
        &self,
        file_name: &str,
        mime: Option<&str>,
        url: &str,
        fallback: &Path,
    ) -> (Option<CategoryId>, PathBuf) {
        let Some(category) = self.category_for(file_name, mime, url) else {
            return (None, fallback.to_path_buf());
        };
        let dir = match &category.settings.save_dir {
            Some(folder) if std::fs::create_dir_all(folder).is_ok() => PathBuf::from(folder),
            _ => fallback.to_path_buf(),
        };
        (Some(category.id), dir)
    }

    /// The first category whose rule matches the file.
    pub(super) fn category_for(
        &self,
        file_name: &str,
        mime: Option<&str>,
        url: &str,
    ) -> Option<CategoryRecord> {
        let categories = self
            .inner
            .store
            .lock()
            .unwrap()
            .list_categories()
            .unwrap_or_default();
        let host = Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(String::from));
        let file = FileFacts {
            file_name,
            mime,
            host: host.as_deref(),
        };
        pick(categories.iter().map(|c| (c, &c.settings.rule)), &file).cloned()
    }
}

/// The last part of a URL's path, as a stand-in name before the server names the file.
pub(super) fn name_from_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut parts| parts.next_back().map(String::from))
        })
        .unwrap_or_default()
}

pub(super) fn find(categories: &[CategoryRecord], name: &str) -> Result<CategoryRecord, String> {
    let wanted = name.trim();
    categories
        .iter()
        .find(|c| c.settings.name.eq_ignore_ascii_case(wanted))
        .cloned()
        .ok_or_else(|| format!("no category named {wanted:?}"))
}
