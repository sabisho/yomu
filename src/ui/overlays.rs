//! Overlay definitions: menus, dialogs, settings tabs, and modal states.

use gpui_kit::component::input::InputState;
use gpui_kit::{Entity, Pixels, Point, SharedString};
use yomu::menu::{BookAction, Entry};

#[derive(Clone)]
pub struct BookMenu {
    pub book_id: String,
    pub target_ids: Vec<String>,
    pub entries: Vec<Entry>,
    pub position: Point<Pixels>,
    pub selected: Option<usize>,
}

#[allow(dead_code)]
pub enum ConfirmAction {
    Book(BookAction, String),
    RemoveBooks(Vec<String>),
    RemoveSelected,
    ResetSettings,
    ClearThumbnailCache,
    ClearAllData,
    DeleteCollection(String),
}

pub struct ConfirmDialog {
    pub title: SharedString,
    pub message: SharedString,
    pub confirm_label: &'static str,
    pub action: ConfirmAction,
}

pub struct PropertiesDialog {
    pub title: SharedString,
    pub rows: Vec<(String, String)>,
}

pub struct SeriesDialog {
    pub book_ids: Vec<String>,
    pub input: Entity<InputState>,
}

pub struct CollectionDialog {
    pub is_new: bool,
    pub collection_id: Option<String>,
    pub input: Entity<InputState>,
}

#[derive(Default)]
pub struct LibraryMenu {
    pub selected: usize,
}

/// At most one menu or dialog is open at a time; it sits on top of whatever view is showing.
#[allow(dead_code)]
pub enum Overlay {
    Menu(BookMenu),
    LibraryMenu(LibraryMenu),
    Confirm(ConfirmDialog),
    Properties(PropertiesDialog),
    Series(SeriesDialog),
    Collection(CollectionDialog),
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    Reader,
    Library,
    Shortcuts,
    Advanced,
}

impl SettingsTab {
    #[allow(dead_code)]
    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::General,
            1 => Self::Appearance,
            2 => Self::Reader,
            3 => Self::Library,
            4 => Self::Shortcuts,
            5 => Self::Advanced,
            _ => Self::General,
        }
    }

    #[allow(dead_code)]
    pub fn to_index(self) -> usize {
        match self {
            Self::General => 0,
            Self::Appearance => 1,
            Self::Reader => 2,
            Self::Library => 3,
            Self::Shortcuts => 4,
            Self::Advanced => 5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Reader => "Reader",
            Self::Library => "Library",
            Self::Shortcuts => "Shortcuts",
            Self::Advanced => "Advanced",
        }
    }
}
