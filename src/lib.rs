//! yomu core: everything that does not need a GUI (archives, decoding, library, import,
//! reader geometry and vim-style key handling).

#![forbid(unsafe_code)]

pub mod archive;
pub mod collections;
pub mod imaging;
pub mod layout;
pub mod library;
pub mod menu;
pub mod motion;
pub mod pagecache;
pub mod scroll;
pub mod search;
pub mod selection;
pub mod util;
pub mod vim;
