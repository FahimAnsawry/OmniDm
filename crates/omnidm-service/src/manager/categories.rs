// SPDX-License-Identifier: GPL-3.0-or-later

//! Categories: which one a download belongs to, and so where a browser download is saved.

use std::path::{Path, PathBuf};

use omnidm_ipc::protocol::{CategoryEdit, CategorySummary};
use omnidm_net::cookies::Url;
use omnidm_queue::category::{CategoryRule, FileFacts, pick};
use omnidm_store::{CategoryId, CategoryRecord, CategorySettings};

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

    /// Adds a category, tried after the others. Returns its id.
    pub fn create_category(&self, edit: CategoryEdit) -> Result<i64, String> {
        let settings = check_category(edit)?;
        let mut store = self.inner.store.lock().unwrap();
        let all = store.list_categories().map_err(|e| e.to_string())?;
        name_is_free(&all, &settings.name, None)?;
        store
            .insert_category(&settings)
            .map(|id| id.0)
            .map_err(|e| e.to_string())
    }

    /// Replaces a category's name, rule and folder.
    pub fn edit_category(&self, id: i64, edit: CategoryEdit) -> Result<(), String> {
        let settings = check_category(edit)?;
        let mut store = self.inner.store.lock().unwrap();
        let all = store.list_categories().map_err(|e| e.to_string())?;
        if !all.iter().any(|c| c.id.0 == id) {
            return Err(format!("no category #{id}"));
        }
        name_is_free(&all, &settings.name, Some(id))?;
        store
            .update_category(CategoryId(id), &settings)
            .map_err(|e| e.to_string())
    }

    /// Deletes a category; its downloads keep their files and lose the category.
    pub fn delete_category(&self, id: i64) -> Result<(), String> {
        let mut store = self.inner.store.lock().unwrap();
        if !matches!(store.get_category(CategoryId(id)), Ok(Some(_))) {
            return Err(format!("no category #{id}"));
        }
        store
            .delete_category(CategoryId(id))
            .map_err(|e| e.to_string())
    }

    /// Moves a category to `index` in the order they are tried (the first match wins).
    pub fn move_category(&self, id: i64, index: usize) -> Result<(), String> {
        let mut store = self.inner.store.lock().unwrap();
        if !matches!(store.get_category(CategoryId(id)), Ok(Some(_))) {
            return Err(format!("no category #{id}"));
        }
        store
            .move_category(CategoryId(id), index)
            .map_err(|e| e.to_string())
    }

    /// The category of a file, by its name, MIME type and URL, and the folder to save it
    /// in: the category's own folder (defaulting to `<fallback>/<category name>`), or
    /// `fallback` when no category matches. The folder is made if it doesn't exist yet.
    pub(super) fn place(
        &self,
        file_name: &str,
        mime: Option<&str>,
        url: &str,
        fallback: &Path,
    ) -> (Option<CategoryId>, PathBuf) {
        let category = self.category_for(file_name, mime, url);
        let wanted = match &category {
            Some(c) => match &c.settings.save_dir {
                Some(folder) => PathBuf::from(folder),
                None => fallback.join(omnidm_net::naming::sanitize_file_name(&c.settings.name)),
            },
            None => fallback.to_path_buf(),
        };
        let dir = if std::fs::create_dir_all(&wanted).is_ok() {
            wanted
        } else {
            // A failure shows when the download starts, with the reason.
            let _ = std::fs::create_dir_all(fallback);
            fallback.to_path_buf()
        };
        (category.map(|c| c.id), dir)
    }

    /// A web link whose name ends in an extension one of the categories lists, such as
    /// `…/setup.exe`: worth offering to download when it is copied.
    pub fn looks_like_file(&self, url: &str) -> bool {
        let Ok(parsed) = Url::parse(url.trim()) else {
            return false;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return false;
        }
        let name = name_from_url(url.trim());
        if !name.contains('.') {
            return false;
        }
        let categories = self
            .inner
            .store
            .lock()
            .unwrap()
            .list_categories()
            .unwrap_or_default();
        let file = FileFacts {
            file_name: &name,
            mime: None,
            host: None,
        };
        pick(categories.iter().map(|c| (c, &c.settings.rule)), &file).is_some()
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

/// Tidies and checks what the user typed for a category.
fn check_category(edit: CategoryEdit) -> Result<CategorySettings, String> {
    let name = edit.name.trim().to_string();
    if name.is_empty() {
        return Err("the category needs a name".into());
    }
    if name.chars().count() > 40 {
        return Err("the category name is too long (40 characters at most)".into());
    }
    let extensions = tidy(edit.extensions, |e| {
        let e = e.trim_start_matches('.').to_lowercase();
        if e.contains(['/', '\\', ' ', '*']) {
            return Err(format!("{e:?} isn't a file extension, like zip or tar.gz"));
        }
        Ok(e)
    })?;
    let mime_types = tidy(edit.mime_types, |m| {
        let m = m.to_lowercase();
        if m.split('/').count() != 2 || m.starts_with('/') || m.ends_with('/') || m.contains(' ') {
            return Err(format!(
                "{m:?} isn't a file type, like video/* or application/pdf"
            ));
        }
        Ok(m)
    })?;
    let hosts = tidy(edit.hosts, |h| {
        let h = h.to_lowercase();
        let h = h
            .strip_prefix("https://")
            .or_else(|| h.strip_prefix("http://"))
            .unwrap_or(&h);
        let h = h.split('/').next().unwrap_or_default();
        let h = h.trim_start_matches("*.").trim_end_matches('.');
        if h.is_empty() || h.contains([' ', '*', '\\', '@']) {
            return Err(format!("{h:?} isn't a site, like example.com"));
        }
        Ok(h.to_string())
    })?;
    let folder = edit
        .folder
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty());
    if let Some(folder) = &folder
        && !Path::new(folder).is_absolute()
    {
        return Err(format!("{folder:?} isn't a full path, like D:\\Videos"));
    }
    Ok(CategorySettings {
        name,
        rule: CategoryRule {
            extensions,
            mime_types,
            hosts,
        },
        save_dir: folder,
    })
}

/// Trims each entry, drops blank ones and repeats, and checks the rest with `check`.
fn tidy(
    entries: Vec<String>,
    check: impl Fn(&str) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    let mut kept: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let entry = check(entry)?;
        if !entry.is_empty() && !kept.contains(&entry) {
            kept.push(entry);
        }
    }
    Ok(kept)
}

fn name_is_free(all: &[CategoryRecord], name: &str, except: Option<i64>) -> Result<(), String> {
    if all
        .iter()
        .any(|c| Some(c.id.0) != except && c.settings.name.eq_ignore_ascii_case(name))
    {
        return Err(format!("there is already a category named {name:?}"));
    }
    Ok(())
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
