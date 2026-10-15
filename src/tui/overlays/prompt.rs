//! A dialog asking for a line of text, e.g. a name for a new playlist.

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear};

use crate::command::Command;
use crate::tui::page::{Ctx, Outcome, Overlay};
use crate::tui::style::{Styles, bold, put, truncate, width};

/// What submitting does with the text.
pub type Submit = Box<dyn FnMut(&str, &Ctx) -> Outcome>;

pub struct Prompt {
    title: String,
    text: String,
    on_submit: Submit,
    area: Rect,
}

impl Prompt {
    pub fn new(
        title: impl Into<String>,
        on_submit: impl FnMut(&str, &Ctx) -> Outcome + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            text: String::new(),
            on_submit: Box::new(on_submit),
            area: Rect::default(),
        }
    }
}

impl Overlay for Prompt {
    fn draw(&mut self, frame: &mut Frame, screen: Rect, ctx: &Ctx) -> Rect {
        let styles = Styles(&ctx.palette);
        let w = (width(&self.title) as u16 + 8)
            .max(40)
            .min(screen.width.saturating_sub(4));
        let area = Rect::new(
            screen.x + screen.width.saturating_sub(w) / 2,
            screen.y + screen.height / 3,
            w,
            5,
        );
        self.area = area;
        frame.render_widget(Clear, area);
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(styles.accent().bg(ctx.palette.statusbar_bg))
            .title(format!(" {} ", self.title))
            .title_style(bold(styles.accent().bg(ctx.palette.statusbar_bg)))
            .style(styles.panel());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let field = Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1);
        let field_style = styles.panel().bg(ctx.palette.highlight_inactive_bg);
        crate::tui::style::fill(frame.buffer_mut(), field, field_style);
        // Show the end of long text, with the cursor after it.
        let room = usize::from(field.width.saturating_sub(1));
        let shown: String = if width(&self.text) > room {
            let skip = self.text.chars().count().saturating_sub(room);
            self.text.chars().skip(skip).collect()
        } else {
            self.text.clone()
        };
        put(
            frame.buffer_mut(),
            field.x,
            field.y,
            &truncate(&shown, room),
            room,
            field_style,
        );
        frame.set_cursor_position((field.x + width(&shown) as u16, field.y));
        area
    }

    fn command(&mut self, cmd: &Command, _ctx: &Ctx) -> Outcome {
        match cmd {
            Command::Back => Outcome::Close,
            _ => Outcome::Consumed,
        }
    }

    fn captures_text(&self) -> bool {
        true
    }

    fn key(&mut self, key: KeyEvent, ctx: &Ctx) -> Outcome {
        match key.code {
            KeyCode::Esc => Outcome::Close,
            KeyCode::Enter if !self.text.trim().is_empty() => {
                let text = self.text.trim().to_string();
                match (self.on_submit)(&text, ctx) {
                    Outcome::Consumed | Outcome::Ignored => Outcome::Close,
                    other => other,
                }
            }
            KeyCode::Backspace => {
                self.text.pop();
                Outcome::Consumed
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.push(c);
                Outcome::Consumed
            }
            _ => Outcome::Consumed,
        }
    }

    fn mouse(&mut self, event: MouseEvent, _ctx: &Ctx) -> Outcome {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left)
                if !self.area.contains((event.column, event.row).into()) =>
            {
                Outcome::Close
            }
            _ => Outcome::Consumed,
        }
    }
}
