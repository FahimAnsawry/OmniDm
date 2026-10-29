// SPDX-License-Identifier: GPL-3.0-or-later

//! Queues, scheduler, categories and power management.
//!
//! The rules here are plain data and arithmetic: when a schedule fires, what happens
//! after a queue finishes, and which category a file belongs to. Running queues lives in
//! `omnidm-service`; storing them in `omnidm-store`.

pub mod category;
pub mod schedule;

use serde::{Deserialize, Serialize};

/// What happens once a queue has nothing left to download.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostAction {
    #[default]
    None,
    Sleep,
    Hibernate,
    Shutdown,
    /// Quit OmniDm.
    Quit,
}

impl PostAction {
    /// The name stored in the database and used by the CLI: `none`, `sleep`…
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
            Self::Shutdown => "shutdown",
            Self::Quit => "quit",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "none" => Self::None,
            "sleep" => Self::Sleep,
            "hibernate" => Self::Hibernate,
            "shutdown" => Self::Shutdown,
            "quit" => Self::Quit,
            _ => return None,
        })
    }
}
