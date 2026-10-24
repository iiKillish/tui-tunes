use std::sync::{Arc, RwLock};

use ratatui::Frame;
use ratatui::crossterm::event::MouseEvent;
use ratatui::layout::Rect;

use crate::command::Command;
use crate::model::folder::{Entry, Folder};
use crate::tui::header::{HeaderAction, PageHeader};
use crate::tui::nav;
use crate::tui::page::{Ctx, Outcome, Page};
use crate::tui::style::songs_and_length;
use crate::tui::widgets::list::ItemList;

/// A folder of music files: its subfolders (Enter goes into one) and its songs.
pub struct FolderPage {
    folder: Arc<Folder>,
    list: ItemList<Entry>,
}

impl FolderPage {
    pub fn new(folder: Arc<Folder>) -> Self {
        let entries = Arc::new(RwLock::new(folder.entries.clone()));
        Self {
            list: ItemList::new(entries).with_source(&folder.name),
            folder,
        }
    }
}

impl Page for FolderPage {
    fn title(&self) -> String {
        self.folder.name.clone()
    }

    fn header(&self, _ctx: &Ctx) -> Option<PageHeader> {
        Some(PageHeader {
            kind: "Folder".into(),
            title: self.folder.name.clone(),
            details: vec![songs_and_length(
                self.folder.tracks().iter().map(|t| t.duration),
            )],
            cover_url: self.folder.cover_url.clone(),
            playable: true,
            liked: None,
        })
    }

    /// Play or shuffle everything in the folder, subfolders too.
    fn header_action(&mut self, action: HeaderAction, ctx: &Ctx) -> Outcome {
        ctx.queue.set_shuffle(action == HeaderAction::Shuffle);
        nav::play_item(Box::new(Entry::Folder(self.folder.clone())), ctx);
        Outcome::Consumed
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx, focused: bool) {
        self.list.draw(frame, area, ctx, focused);
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        self.list.command(cmd, ctx)
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        self.list.mouse(event, ctx)
    }
}
