//! Bounded LRU cache for library and card cover images.
//!
//! Provides a bounded capacity implementation of GPUI's [`ImageCache`] trait.
//! When capacity is exceeded, the least-recently-used cover textures are evicted
//! and explicitly freed from window atlas textures via [`Window::drop_image`].

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui_kit::{
    App, AppContext, Entity, ImageCache, ImageCacheError, ImageCacheItem, RenderImage, Resource,
    Window, hash,
};

/// Default capacity for the bounded cover cache (256 entries $\approx 138$ MB at 300x450 BGRA8).
pub const DEFAULT_COVER_CACHE_CAPACITY: usize = 256;

/// Bounded LRU image cache for covers.
pub struct BoundedCoverCache {
    items: HashMap<u64, ImageCacheItem>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl BoundedCoverCache {
    /// Create a new bounded cover cache entity with the specified capacity.
    pub fn new(capacity: usize, cx: &mut App) -> Entity<Self> {
        let capacity = capacity.max(16);
        let cache = cx.new(|_cx| Self {
            items: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
        });

        cx.observe_release(&cache, |this, cx| {
            this.clear_without_window(cx);
        })
        .detach();

        cache
    }

    /// Load an image using LRU tracking and bounded eviction.
    pub fn load(
        &mut self,
        source: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(source);

        // Cache hit: bump key to the back of order (most recently used)
        if let Some(item) = self.items.get(&key) {
            if let Some(pos) = self.order.iter().position(|&k| k == key) {
                self.order.remove(pos);
            }
            self.order.push_back(key);
            return item.use_image(window);
        }

        self.make_room(Some(window), cx);

        let new_item = ImageCacheItem::new(source, cx);
        let result = new_item.use_image(window);
        self.items.insert(key, new_item);
        self.order.push_back(key);

        result
    }

    /// Insert an image cache item directly without requiring a window layout/paint context.
    #[allow(dead_code)]
    pub fn insert_item(&mut self, key: u64, item: ImageCacheItem, cx: &mut App) {
        self.remove_key(key, None, cx);
        self.make_room(None, cx);
        self.items.insert(key, item);
        self.order.push_back(key);
    }

    fn make_room(&mut self, window: Option<&mut Window>, cx: &mut App) {
        let mut window = window;
        while self.items.len() >= self.capacity {
            let Some(oldest_key) = self.order.pop_front() else {
                break;
            };
            if let Some(item) = self.items.remove(&oldest_key) {
                Self::drop_item(item, window.as_deref_mut(), cx);
            }
        }
    }

    fn remove_key(&mut self, key: u64, window: Option<&mut Window>, cx: &mut App) {
        if let Some(position) = self.order.iter().position(|&candidate| candidate == key) {
            self.order.remove(position);
        }
        if let Some(item) = self.items.remove(&key) {
            Self::drop_item(item, window, cx);
        }
    }

    fn drop_item(item: ImageCacheItem, window: Option<&mut Window>, cx: &mut App) {
        if let Some(Ok(image)) = item.get() {
            cx.drop_image(image, window);
        }
    }

    fn clear_items(&mut self, window: Option<&mut Window>, cx: &mut App) {
        let mut window = window;
        for (_, item) in self.items.drain() {
            Self::drop_item(item, window.as_deref_mut(), cx);
        }
        self.order.clear();
    }

    /// Number of items currently tracked in the cache.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the cache contains zero items.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Configured maximum capacity.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Remove a specific image source from the cache if present.
    #[allow(dead_code)]
    pub fn remove(&mut self, source: &Resource, window: &mut Window, cx: &mut App) {
        self.remove_key(hash(source), Some(window), cx);
    }

    /// Clear all cached items and drop their GPU textures.
    #[allow(dead_code)]
    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        self.clear_items(Some(window), cx);
    }

    /// Clear all cached items and release their textures without a current window.
    pub fn clear_without_window(&mut self, cx: &mut App) {
        self.clear_items(None, cx);
    }
}

impl ImageCache for BoundedCoverCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        BoundedCoverCache::load(self, resource, window, cx)
    }
}
