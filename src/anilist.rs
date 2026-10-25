//! AniList GraphQL API client, metadata cache, and offline synchronization.
//!
//! Provides manga search, metadata retrieval, user list synchronization, and
//! offline queue management in compliance with AniList API guidelines.

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

pub const ANILIST_GRAPHQL_URL: &str = "https://graphql.anilist.co";
pub const ANILIST_RATE_LIMIT_PER_MINUTE: usize = 90;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MediaListStatus {
    Current,
    Planning,
    Completed,
    Dropped,
    Paused,
    Repeating,
}

impl MediaListStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Current => "Reading",
            Self::Planning => "Planning",
            Self::Completed => "Completed",
            Self::Dropped => "Dropped",
            Self::Paused => "Paused",
            Self::Repeating => "Rereading",
        }
    }

    pub fn from_str_lenient(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "reading" | "current" => Some(Self::Current),
            "planning" | "plan to read" => Some(Self::Planning),
            "completed" => Some(Self::Completed),
            "dropped" => Some(Self::Dropped),
            "paused" | "on hold" => Some(Self::Paused),
            "repeating" | "rereading" => Some(Self::Repeating),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AniListTitle {
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
}

impl AniListTitle {
    pub fn preferred(&self) -> &str {
        if let Some(en) = &self.english
            && !en.trim().is_empty()
        {
            return en.as_str();
        }
        if let Some(rom) = &self.romaji
            && !rom.trim().is_empty()
        {
            return rom.as_str();
        }
        if let Some(nat) = &self.native
            && !nat.trim().is_empty()
        {
            return nat.as_str();
        }
        "Untitled Manga"
    }

    pub fn subtitle(&self) -> Option<&str> {
        if let Some(nat) = &self.native
            && !nat.trim().is_empty()
        {
            return Some(nat.as_str());
        }
        if let Some(rom) = &self.romaji
            && self.english.is_some()
            && !rom.trim().is_empty()
        {
            return Some(rom.as_str());
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AniListMedia {
    pub id: u64,
    pub title: AniListTitle,
    pub cover_image_extra_large: Option<String>,
    pub cover_image_large: Option<String>,
    pub banner_image: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub start_year: Option<i32>,
    pub chapters: Option<i32>,
    pub volumes: Option<i32>,
    pub average_score: Option<i32>,
    pub popularity: Option<i32>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub author: Option<String>,
    pub artist: Option<String>,
    pub serialization: Option<String>,
    pub user_status: Option<MediaListStatus>,
    pub user_progress: Option<i32>,
    pub site_url: Option<String>,
}

impl AniListMedia {
    pub fn clean_description(&self) -> String {
        let Some(raw) = &self.description else {
            return "No synopsis available.".to_string();
        };
        // Strip basic HTML tags returned by AniList GraphQL (e.g. <br>, <i>, </i>)
        let mut clean = String::with_capacity(raw.len());
        let mut in_tag = false;
        for c in raw.chars() {
            if c == '<' {
                in_tag = true;
            } else if c == '>' {
                in_tag = false;
            } else if !in_tag {
                clean.push(c);
            }
        }
        clean.replace("&quot;", "\"").replace("&amp;", "&")
    }

    pub fn cover_url(&self) -> Option<&str> {
        self.cover_image_extra_large
            .as_deref()
            .or(self.cover_image_large.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AniListUser {
    pub id: u64,
    pub name: String,
    pub avatar_url: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedSyncItem {
    pub media_id: u64,
    pub progress: i32,
    pub status: Option<MediaListStatus>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AniListData {
    pub user: Option<AniListUser>,
    pub matches: HashMap<String, u64>, // local book id -> AniList media id
    pub cache: HashMap<u64, AniListMedia>,
    pub sync_queue: Vec<QueuedSyncItem>,
    pub last_synced: Option<u64>,
}

impl AniListData {
    pub fn load_or_default(path: &Path) -> Self {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(data) = serde_json::from_str::<Self>(&text)
        {
            return data;
        }
        Self::default()
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.user
            .as_ref()
            .is_some_and(|u| u.token.is_some() || !u.name.is_empty())
    }

    pub fn user_name(&self) -> &str {
        self.user
            .as_ref()
            .map(|u| u.name.as_str())
            .unwrap_or("Guest")
    }

    pub fn disconnect(&mut self) {
        self.user = None;
    }

    pub fn get_media_for_book(&self, book_id: &str) -> Option<&AniListMedia> {
        let media_id = self.matches.get(book_id)?;
        self.cache.get(media_id)
    }

    pub fn match_book(&mut self, book_id: String, media: AniListMedia) {
        let media_id = media.id;
        self.cache.insert(media_id, media);
        self.matches.insert(book_id, media_id);
    }

    pub fn queue_progress_update(
        &mut self,
        media_id: u64,
        progress: i32,
        status: Option<MediaListStatus>,
    ) {
        if let Some(media) = self.cache.get_mut(&media_id) {
            media.user_progress = Some(progress);
            if status.is_some() {
                media.user_status = status;
            }
        }
        self.sync_queue.retain(|item| item.media_id != media_id);
        self.sync_queue.push(QueuedSyncItem {
            media_id,
            progress,
            status,
            timestamp: crate::util::now_secs(),
        });
    }

    pub fn count_by_status(&self, status: Option<MediaListStatus>) -> usize {
        if let Some(s) = status {
            self.cache
                .values()
                .filter(|m| m.user_status == Some(s))
                .count()
        } else {
            self.matches.len()
        }
    }
}

#[derive(Clone)]
pub struct AniListClient {
    agent: ureq::Agent,
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

struct RateLimiter {
    requests: Vec<Instant>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
        }
    }

    fn check_and_wait(&mut self) {
        let now = Instant::now();
        self.requests
            .retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        if self.requests.len() >= ANILIST_RATE_LIMIT_PER_MINUTE {
            let oldest = self.requests[0];
            let elapsed = now.duration_since(oldest);
            if elapsed < Duration::from_secs(60) {
                std::thread::sleep(Duration::from_secs(60) - elapsed);
            }
        }
        self.requests.push(Instant::now());
    }
}

impl Default for AniListClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AniListClient {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(15))
            .build();
        Self {
            agent,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new())),
        }
    }

    pub fn search_manga(&self, query: &str) -> anyhow::Result<Vec<AniListMedia>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        self.rate_limiter.lock().unwrap().check_and_wait();

        let graphql = r#"
            query ($search: String) {
                Page (page: 1, perPage: 12) {
                    media (search: $search, type: MANGA, sort: SEARCH_MATCH) {
                        id
                        title {
                            romaji
                            english
                            native
                        }
                        coverImage {
                            extraLarge
                            large
                        }
                        bannerImage
                        description
                        status
                        startDate {
                            year
                        }
                        chapters
                        volumes
                        averageScore
                        popularity
                        genres
                        siteUrl
                        staff (sort: RELEVANCE) {
                            edges {
                                role
                                node {
                                    name {
                                        full
                                    }
                                }
                            }
                        }
                    }
                }
            }
        "#;

        let payload = serde_json::json!({
            "query": graphql,
            "variables": {
                "search": query
            }
        });

        let resp: serde_json::Value = self
            .agent
            .post(ANILIST_GRAPHQL_URL)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_json(payload)?
            .into_json()?;

        let mut results = Vec::new();
        let Some(media_list) = resp["data"]["Page"]["media"].as_array() else {
            return Ok(results);
        };

        for item in media_list {
            results.push(parse_media_json(item));
        }

        Ok(results)
    }

    pub fn fetch_media_by_id(&self, id: u64) -> anyhow::Result<AniListMedia> {
        self.rate_limiter.lock().unwrap().check_and_wait();

        let graphql = r#"
            query ($id: Int) {
                Media (id: $id, type: MANGA) {
                    id
                    title {
                        romaji
                        english
                        native
                    }
                    coverImage {
                        extraLarge
                        large
                    }
                    bannerImage
                    description
                    status
                    startDate {
                        year
                    }
                    chapters
                    volumes
                    averageScore
                    popularity
                    genres
                    siteUrl
                    staff (sort: RELEVANCE) {
                        edges {
                            role
                            node {
                                name {
                                    full
                                }
                            }
                        }
                    }
                }
            }
        "#;

        let payload = serde_json::json!({
            "query": graphql,
            "variables": {
                "id": id
            }
        });

        let resp: serde_json::Value = self
            .agent
            .post(ANILIST_GRAPHQL_URL)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_json(payload)?
            .into_json()?;

        let media_json = &resp["data"]["Media"];
        if media_json.is_null() {
            anyhow::bail!("Media not found on AniList for ID {}", id);
        }

        Ok(parse_media_json(media_json))
    }

    pub fn fetch_user_lists(&self, username: &str) -> anyhow::Result<Vec<AniListMedia>> {
        self.rate_limiter.lock().unwrap().check_and_wait();

        let graphql = r#"
            query ($userName: String) {
                MediaListCollection (userName: $userName, type: MANGA) {
                    lists {
                        name
                        status
                        entries {
                            status
                            progress
                            media {
                                id
                                title {
                                    romaji
                                    english
                                    native
                                }
                                coverImage {
                                    extraLarge
                                    large
                                }
                                bannerImage
                                description
                                status
                                startDate {
                                    year
                                }
                                chapters
                                volumes
                                averageScore
                                popularity
                                genres
                                siteUrl
                            }
                        }
                    }
                }
            }
        "#;

        let payload = serde_json::json!({
            "query": graphql,
            "variables": {
                "userName": username
            }
        });

        let resp: serde_json::Value = self
            .agent
            .post(ANILIST_GRAPHQL_URL)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_json(payload)?
            .into_json()?;

        let mut all_media = Vec::new();
        if let Some(lists) = resp["data"]["MediaListCollection"]["lists"].as_array() {
            for list in lists {
                if let Some(entries) = list["entries"].as_array() {
                    for entry in entries {
                        let mut media = parse_media_json(&entry["media"]);
                        if let Some(status_str) = entry["status"].as_str() {
                            media.user_status = MediaListStatus::from_str_lenient(status_str);
                        }
                        if let Some(prog) = entry["progress"].as_i64() {
                            media.user_progress = Some(prog as i32);
                        }
                        all_media.push(media);
                    }
                }
            }
        }

        Ok(all_media)
    }

    pub fn sync_queued_updates(
        &self,
        token: &str,
        queue: &[QueuedSyncItem],
    ) -> anyhow::Result<usize> {
        let mut synced_count = 0;
        let mutation = r#"
            mutation ($mediaId: Int, $progress: Int, $status: MediaListStatus) {
                SaveMediaListEntry (mediaId: $mediaId, progress: $progress, status: $status) {
                    id
                    status
                    progress
                }
            }
        "#;

        for item in queue {
            self.rate_limiter.lock().unwrap().check_and_wait();
            let mut variables = serde_json::Map::new();
            variables.insert("mediaId".to_string(), serde_json::json!(item.media_id));
            variables.insert("progress".to_string(), serde_json::json!(item.progress));
            if let Some(status) = &item.status {
                variables.insert("status".to_string(), serde_json::json!(status));
            }

            let payload = serde_json::json!({
                "query": mutation,
                "variables": variables
            });

            let res = self
                .agent
                .post(ANILIST_GRAPHQL_URL)
                .set("Content-Type", "application/json")
                .set("Accept", "application/json")
                .set("Authorization", &format!("Bearer {token}"))
                .send_json(payload);

            if res.is_ok() {
                synced_count += 1;
            }
        }

        Ok(synced_count)
    }
}

fn parse_media_json(val: &serde_json::Value) -> AniListMedia {
    let id = val["id"].as_u64().unwrap_or(0);
    let title = AniListTitle {
        romaji: val["title"]["romaji"].as_str().map(String::from),
        english: val["title"]["english"].as_str().map(String::from),
        native: val["title"]["native"].as_str().map(String::from),
    };
    let cover_image_extra_large = val["coverImage"]["extraLarge"].as_str().map(String::from);
    let cover_image_large = val["coverImage"]["large"].as_str().map(String::from);
    let banner_image = val["bannerImage"].as_str().map(String::from);
    let description = val["description"].as_str().map(String::from);
    let status = val["status"].as_str().map(String::from);
    let start_year = val["startDate"]["year"].as_i64().map(|y| y as i32);
    let chapters = val["chapters"].as_i64().map(|c| c as i32);
    let volumes = val["volumes"].as_i64().map(|v| v as i32);
    let average_score = val["averageScore"].as_i64().map(|s| s as i32);
    let popularity = val["popularity"].as_i64().map(|p| p as i32);
    let site_url = val["siteUrl"].as_str().map(String::from);

    let genres = val["genres"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|g| g.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut author = None;
    let mut artist = None;
    if let Some(staff) = val["staff"]["edges"].as_array() {
        for edge in staff {
            let role = edge["role"].as_str().unwrap_or("");
            let name = edge["node"]["name"]["full"].as_str().unwrap_or("");
            if (role.contains("Story")
                || role.contains("Original Creator")
                || role.contains("Author"))
                && author.is_none()
            {
                author = Some(name.to_string());
            }
            if (role.contains("Art") || role.contains("Illustration")) && artist.is_none() {
                artist = Some(name.to_string());
            }
        }
    }

    AniListMedia {
        id,
        title,
        cover_image_extra_large,
        cover_image_large,
        banner_image,
        description,
        status,
        start_year,
        chapters,
        volumes,
        average_score,
        popularity,
        genres,
        tags: Vec::new(),
        author,
        artist,
        serialization: Some("Serialized".to_string()),
        user_status: None,
        user_progress: None,
        site_url,
    }
}

/// Computes a normalized match score between a comic title and an AniList entry title.
pub fn title_match_score(local_title: &str, candidate: &AniListMedia) -> f32 {
    let local_norm = normalize_for_match(local_title);
    if local_norm.is_empty() {
        return 0.0;
    }

    let mut best_score = 0.0f32;
    for cand_title in [
        candidate.title.english.as_deref(),
        candidate.title.romaji.as_deref(),
        candidate.title.native.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let cand_norm = normalize_for_match(cand_title);
        if cand_norm == local_norm {
            return 1.0;
        }
        if cand_norm.contains(&local_norm) || local_norm.contains(&cand_norm) {
            let ratio = (local_norm.len().min(cand_norm.len()) as f32)
                / (local_norm.len().max(cand_norm.len()) as f32);
            best_score = best_score.max(0.7 + ratio * 0.25);
        }
    }

    best_score
}

fn normalize_for_match(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extracts an AniList media ID from a user input string (either raw digits or full URL).
pub fn parse_anilist_id(input: &str) -> Option<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(pos) = trimmed.find("/manga/") {
        let after = &trimmed[pos + 7..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(id) = digits.parse::<u64>()
            && id > 0
        {
            return Some(id);
        }
    }
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if let Ok(id) = digits.parse::<u64>()
        && id > 0
    {
        return Some(id);
    }
    None
}
