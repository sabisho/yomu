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
            for (_, item) in this.items.drain() {
                if let Some(Ok(image)) = item.get() {
                    cx.drop_image(image, None);
                }
            }
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

        // Cache miss: evict oldest entry if at capacity
        while self.items.len() >= self.capacity && !self.order.is_empty() {
            if let Some(oldest_key) = self.order.pop_front()
                && let Some(evicted) = self.items.remove(&oldest_key)
                && let Some(Ok(image)) = evicted.get()
            {
                cx.drop_image(image, Some(window));
            }
        }

        let new_item = ImageCacheItem::new(source, cx);
        let result = new_item.use_image(window);
        self.items.insert(key, new_item);
        self.order.push_back(key);

        result
    }

    /// Insert an image cache item directly without requiring a window layout/paint context.
    #[allow(dead_code)]
    pub fn insert_item(&mut self, key: u64, item: ImageCacheItem, cx: &mut App) {
        while self.items.len() >= self.capacity && !self.order.is_empty() {
            if let Some(oldest_key) = self.order.pop_front()
                && let Some(evicted) = self.items.remove(&oldest_key)
                && let Some(Ok(image)) = evicted.get()
            {
                cx.drop_image(image, None);
            }
        }
        self.items.insert(key, item);
        self.order.push_back(key);
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
        let key = hash(source);
        if let Some(pos) = self.order.iter().position(|&k| k == key) {
            self.order.remove(pos);
        }
        if let Some(item) = self.items.remove(&key)
            && let Some(Ok(image)) = item.get()
        {
            cx.drop_image(image, Some(window));
        }
    }

    /// Clear all cached items and drop their GPU textures.
    #[allow(dead_code)]
    pub fn clear(&mut self, window: &mut Window, cx: &mut App) {
        for (_, item) in self.items.drain() {
            if let Some(Ok(image)) = item.get() {
                cx.drop_image(image, Some(window));
            }
        }
        self.order.clear();
    }

    /// Clear all cached items without a window handle (e.g. during background data reset).
    pub fn clear_without_window(&mut self) {
        self.items.clear();
        self.order.clear();
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
