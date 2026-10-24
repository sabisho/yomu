//! Manga collections: default smart categories and user-defined custom collections.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub book_ids: Vec<String>,
    #[serde(default)]
    pub created_at: u64,
}

impl Collection {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            book_ids: Vec::new(),
            created_at: crate::util::now_secs(),
        }
    }

    pub fn contains(&self, book_id: &str) -> bool {
        self.book_ids.iter().any(|id| id == book_id)
    }

    pub fn add(&mut self, book_id: impl Into<String>) {
        let id = book_id.into();
        if !self.contains(&id) {
            self.book_ids.push(id);
        }
    }

    pub fn remove(&mut self, book_id: &str) {
        self.book_ids.retain(|id| id != book_id);
    }
}
