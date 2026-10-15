use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use cursive::align::HAlign;
use cursive::direction::Direction;
use cursive::event::{AnyCb, Event, EventResult, Key, MouseButton, MouseEvent};
use cursive::theme::{ColorStyle, ColorType, Theme};
use cursive::traits::View;
use cursive::vec::Vec2;
use cursive::view::{CannotFocus, IntoBoxedView, Selector};
use cursive::views::EditView;
use cursive::{Cursive, Printer};
use unicode_width::UnicodeWidthStr;

use crate::application::UserData;
use crate::command::{self, ALL_COMMANDS, Command, JumpMode};
use crate::commands::CommandResult;
use crate::config::{self, Config};
use crate::events;
use crate::ext_traits::CursiveExt;
use crate::traits::{IntoBoxedViewExt, ViewExt};
use crate::ui::nowplaying::{self, NowPlaying};
use crate::ui::queuepanel::QueuePanel;

/// Narrowest the screen content gets when showing the queue panel; below this the panel hides.
const MIN_CONTENT_WIDTH_WITH_QUEUE_PANEL: usize = 50;

/// Smallest number of content rows to keep when showing the player bar.
const MIN_CONTENT_ROWS_WITH_NOW_PLAYING: usize = 8;
/// Height of the status bar, used instead of the player bar when that's disabled or doesn't fit.
const STATUSBAR_HEIGHT: usize = 2;

pub struct Layout {
    screens: HashMap<String, Box<dyn ViewExt>>,
    stack: HashMap<String, Vec<Box<dyn ViewExt>>>,
    statusbar: Box<dyn View>,
    now_playing: Option<NowPlaying>,
    queue_panel: Option<QueuePanel>,
    focus: Option<String>,
    previous_focus: Option<String>,
    cmdline: EditView,
    cmdline_focus: bool,
    result: Result<Option<String>, String>,
    result_time: Option<SystemTime>,
    last_size: Vec2,
    ev: events::EventManager,
    theme: Theme,
    configuration: Arc<Config>,
    /// Pending count prefix, cleared when a prompt opens so it can't leak onto
    /// the next keypress.
    pending_count: Arc<AtomicUsize>,
}

impl Layout {
    pub fn new<T: IntoBoxedView>(
        status: T,
        ev: &events::EventManager,
        theme: Theme,
        configuration: Arc<Config>,
        pending_count: Arc<AtomicUsize>,
    ) -> Self {
        let style = ColorStyle::new(
            ColorType::Color(*theme.palette.custom("cmdline").unwrap()),
            ColorType::Color(*theme.palette.custom("cmdline_bg").unwrap()),
        );
        let mut command_line_input = EditView::new().filler(" ").style(style);

        let event_manager = ev.clone();
        // 1. When a search was submitted on the commandline...
        command_line_input.set_on_submit(move |s, cmd| {
            // 2. Clear the commandline on Layout...
            s.on_layout(|_, mut layout| layout.clear_cmdline());

            // 3. Get the actual command without the prefix (like `:` or `/`)...
            let mut command_characters = cmd.chars();
            command_characters.next();
            let cmd_without_prefix = command_characters.as_str();
            if cmd.strip_prefix('/').is_some() {
                // 4. If it is a search command...

                // 5. Send a jump command with the search query to the command manager.
                let command = Command::Jump(JumpMode::Query(cmd_without_prefix.to_string()));
                if let Some(data) = s.user_data::<UserData>().cloned() {
                    data.cmd.handle(s, command);
                }
            } else {
                // 4. If it is an actual command...

                // 5. Parse the command and...
                match command::parse(cmd_without_prefix) {
                    Ok(commands) => {
                        // 6. Send the parsed command to the command manager.
                        if let Some(data) = s.user_data::<UserData>().cloned() {
                            for cmd in commands {
                                data.cmd.handle(s, cmd);
                            }
                        }
                    }
                    Err(err) => {
                        // 6. Set an error message on the global layout.
                        s.on_layout(|_, mut layout| layout.set_result(Err(err.to_string())));
                    }
                }
            }
            event_manager.trigger();
        });

        Self {
            screens: HashMap::new(),
            stack: HashMap::new(),
            statusbar: status.into_boxed_view(),
            now_playing: None,
            queue_panel: None,
            focus: None,
            previous_focus: None,
            cmdline: command_line_input,
            cmdline_focus: false,
            result: Ok(None),
            result_time: None,
            last_size: Vec2::new(0, 0),
            ev: ev.clone(),
            theme,
            configuration,
            pending_count,
        }
    }

    /// Show the player bar `view` at the bottom instead of the status bar.
    pub fn now_playing(mut self, view: NowPlaying) -> Self {
        self.now_playing = Some(view);
        self
    }

    /// Show the queue panel `view` on the right of the screen content.
    pub fn queue_panel(mut self, view: QueuePanel) -> Self {
        self.queue_panel = Some(view);
        self
    }

    /// Columns taken by the queue panel in a layout `total_width` wide: 0 when it's disabled, on
    /// the queue screen (which shows the whole queue) or when it would squeeze the content.
    fn queue_panel_width(&self, total_width: usize) -> usize {
        match &self.queue_panel {
            Some(panel)
                if panel.width() > 0
                    && self.focus.as_deref() != Some("queue")
                    && total_width >= panel.width() + 1 + MIN_CONTENT_WIDTH_WITH_QUEUE_PANEL =>
            {
                panel.width()
            }
            _ => 0,
        }
    }

    /// Width of the screen content: everything left of the queue panel and its 1-column gap.
    fn content_width(&self, total_width: usize) -> usize {
        match self.queue_panel_width(total_width) {
            0 => total_width,
            panel => total_width - panel - 1,
        }
    }

    /// Rows taken by the command line and result message at the bottom.
    fn cmdline_height(&self) -> usize {
        let mut height = usize::from(!self.cmdline.get_content().is_empty());
        if self
            .get_result()
            .as_ref()
            .map(Option::is_some)
            .unwrap_or(true)
        {
            height += 1;
        }
        height
    }

    /// Whether the player bar replaces the status bar in a layout of `size`: it must be enabled
    /// and the window big enough for it.
    fn now_playing_shown(&self, size: Vec2) -> bool {
        self.now_playing.as_ref().is_some_and(|bar| {
            let height = bar.height();
            height > 0
                && size.x >= nowplaying::MIN_WIDTH
                && size.y >= height + 1 + MIN_CONTENT_ROWS_WITH_NOW_PLAYING
        })
    }

    /// Rows below the screen content: the player bar or the status bar.
    fn bottom_height(&self, size: Vec2) -> usize {
        match &self.now_playing {
            Some(bar) if self.now_playing_shown(size) => bar.height(),
            _ => STATUSBAR_HEIGHT,
        }
    }

    /// Draw the player bar's images in the terminal; see [NowPlaying::render_to_terminal].
    #[cfg(feature = "cover")]
    pub fn render_now_playing(&self, overlays: &[crate::ui::cover::Area]) {
        if let Some(bar) = &self.now_playing {
            bar.render_to_terminal(overlays);
        }
    }

    pub fn enable_cmdline(&mut self, prefix: char) {
        if !self.cmdline_focus {
            self.pending_count.store(0, Ordering::Relaxed);
            self.cmdline.set_content(prefix);
            self.cmdline_focus = true;
        }
    }

    pub fn enable_jump(&mut self) {
        if !self.cmdline_focus {
            self.pending_count.store(0, Ordering::Relaxed);
            self.cmdline.set_content("/");
            self.cmdline_focus = true;
        }
    }

    pub fn add_screen<S: Into<String>, T: IntoBoxedViewExt>(&mut self, id: S, view: T) {
        if let Some(view) = self.get_top_view() {
            view.on_leave();
        }

        let s = id.into();
        self.screens.insert(s.clone(), view.into_boxed_view_ext());
        self.stack.insert(s.clone(), Vec::new());
        self.focus = Some(s);
    }

    pub fn screen<S: Into<String>, T: IntoBoxedViewExt>(mut self, id: S, view: T) -> Self {
        self.add_screen(id, view);
        self
    }

    pub fn has_screen(&self, id: &str) -> bool {
        self.screens.contains_key(id)
    }

    pub fn set_screen<S: Into<String>>(&mut self, id: S) {
        if let Some(view) = self.get_top_view() {
            view.on_leave();
        }

        let s = id.into();
        if let Some(current_focus) = &self.focus
            && current_focus != &s
        {
            self.previous_focus = Some(current_focus.clone());
        }
        self.focus = Some(s);
        self.cmdline_focus = false;

        // trigger a redraw
        self.ev.trigger();
    }

    pub fn set_result(&mut self, result: Result<Option<String>, String>) {
        self.result = result;
        self.result_time = Some(SystemTime::now());
    }

    pub fn clear_cmdline(&mut self) {
        self.cmdline.set_content("");
        self.cmdline_focus = false;
        self.result = Ok(None);
        self.result_time = None;
    }

    fn get_result(&self) -> Result<Option<String>, String> {
        if let Some(t) = self.result_time
            && t.elapsed().unwrap() > Duration::from_secs(5)
        {
            return Ok(None);
        }
        self.result.clone()
    }

    pub fn push_view(&mut self, view: Box<dyn ViewExt>) {
        if let Some(view) = self.get_top_view() {
            view.on_leave();
        }

        if let Some(stack) = self.get_focussed_stack_mut() {
            stack.push(view)
        }
    }

    pub fn pop_view(&mut self) {
        if let Some(view) = self.get_top_view() {
            view.on_leave();
        }

        self.get_focussed_stack_mut().map(|stack| stack.pop());
    }

    fn back(&mut self) {
        if !self.is_current_stack_empty() {
            self.pop_view();
            return;
        }

        if self.focus.as_deref() == Some("search")
            && let Some(previous_focus) = self.previous_focus.clone()
            && self.screens.contains_key(&previous_focus)
        {
            self.set_screen(previous_focus);
        }
    }

    #[allow(clippy::borrowed_box)]
    fn get_current_screen(&self) -> Option<&Box<dyn ViewExt>> {
        self.focus
            .as_ref()
            .and_then(|focus| self.screens.get(focus))
    }

    fn get_focussed_stack_mut(&mut self) -> Option<&mut Vec<Box<dyn ViewExt>>> {
        let focus = self.focus.clone();
        if let Some(focus) = &focus {
            self.stack.get_mut(focus)
        } else {
            None
        }
    }

    fn get_focussed_stack(&self) -> Option<&Vec<Box<dyn ViewExt>>> {
        self.focus.as_ref().and_then(|focus| self.stack.get(focus))
    }

    fn is_current_stack_empty(&self) -> bool {
        self.get_focussed_stack()
            .map(|s| s.is_empty())
            .unwrap_or(false)
    }

    #[allow(clippy::borrowed_box)]
    fn get_top_view(&self) -> Option<&Box<dyn ViewExt>> {
        let focussed_stack = self.get_focussed_stack();
        if focussed_stack.map(|s| s.len()).unwrap_or_default() > 0 {
            focussed_stack.unwrap().last()
        } else if let Some(id) = &self.focus {
            self.screens.get(id)
        } else {
            None
        }
    }

    fn get_current_view_mut(&mut self) -> Option<&mut Box<dyn ViewExt>> {
        if let Some(focus) = &self.focus {
            let last_view = self
                .stack
                .get_mut(focus)
                .filter(|stack| !stack.is_empty())
                .and_then(|stack| stack.last_mut());
            if last_view.is_some() {
                last_view
            } else {
                self.screens.get_mut(focus)
            }
        } else {
            None
        }
    }

    /// Propagate the given event to the command line.
    fn command_line_handle_event(&mut self, event: Event) -> EventResult {
        let is_left_right_event = matches!(event, Event::Key(Key::Left) | Event::Key(Key::Right));
        let result = match event {
            // [Key::Tab] isn't actually handled by EditView internally.
            Event::Key(Key::Tab) => self.command_completion(),
            _ => self.cmdline.on_event(event),
        };

        if self.cmdline.get_content().is_empty() {
            self.clear_cmdline();
        }

        if is_left_right_event {
            EventResult::consumed()
        } else {
            result
        }
    }

    /// Completes the command line when the `cmdline` input uniquely matches a command.
    ///
    /// TODO: Add support for aliases.
    ///
    /// # Returns
    ///
    /// [EventResult::Consumed]
    fn command_completion(&mut self) -> EventResult {
        let content = self.cmdline.get_content();

        // Strip the prefix from the content and store it for further processing.
        let mut content_chars = content.chars();
        let prefix = content_chars.next();
        let command_without_prefix = content_chars.as_str();

        let command = word_completion(command_without_prefix, ALL_COMMANDS);
        let _ = match prefix {
            Some(prefix) => self.cmdline.set_content(format!("{prefix}{command}")),
            None => self.cmdline.set_content(command),
        };
        EventResult::consumed()
    }
}

impl View for Layout {
    fn draw(&self, printer: &Printer<'_, '_>) {
        // Clear everything first. Views only paint the rows they use (a short list leaves the
        // rest of its area alone), and the bottom bar moves when the command line or a result
        // message appears, so anything not repainted would keep whatever was drawn there before.
        printer.with_color(ColorStyle::primary(), |printer| {
            for y in 0..printer.size.y {
                printer.print_hline((0, y), printer.size.x, " ");
            }
        });

        let result = self.get_result();

        let cmdline_visible = !self.cmdline.get_content().is_empty();
        let mut cmdline_height = usize::from(cmdline_visible);
        if result.as_ref().map(Option::is_some).unwrap_or(true) {
            cmdline_height += 1;
        }

        let screen_title = self
            .get_current_screen()
            .map(|screen| screen.title())
            .unwrap_or_default();

        let bottom_height = self.bottom_height(printer.size);
        let main_rows = printer.size.y - bottom_height - cmdline_height;

        // Queue panel on the right, from the top down to the bottom bar.
        let panel_width = self.queue_panel_width(printer.size.x);
        if let Some(panel) = &self.queue_panel
            && panel_width > 0
        {
            panel.draw(
                &printer
                    .offset((printer.size.x - panel_width, 0))
                    .cropped((panel_width, main_rows)),
            );
        }

        let full_printer = printer;
        let printer = &printer.cropped((self.content_width(printer.size.x), printer.size.y));

        if let Some(view) = self.get_top_view() {
            // Title centered, with the back button on the left and the subtitle on the right,
            // each shortened or dropped rather than overlapping the title.
            let width = printer.size.x;
            let title = view.title();
            let title_width = title.width().min(width);
            let title_x = HAlign::Center.get_offset(title_width, width);
            printer.with_color(ColorStyle::title_primary(), |printer| {
                printer.print((title_x, 0), &truncate_to_width(&title, width));
            });

            if !self.is_current_stack_empty() {
                let back = format!("< {screen_title}");
                let room = title_x.saturating_sub(2);
                if room >= 1 {
                    printer.with_color(ColorStyle::title_secondary(), |printer| {
                        printer.print((1, 0), &truncate_to_width(&back, room));
                    });
                }
            }

            let title_sub = view.title_sub();
            let sub_x = HAlign::Right.get_offset(title_sub.width(), width);
            if sub_x > title_x + title_width {
                printer.with_color(ColorStyle::secondary(), |printer| {
                    printer.print((sub_x, 0), &title_sub);
                });
            }

            // screen content
            let printer = &printer
                .offset((0, 1))
                .cropped((
                    printer.size.x,
                    printer.size.y - 1 - bottom_height - cmdline_height,
                ))
                .focused(true);
            view.draw(printer);
        }

        let printer = full_printer;
        let bottom = printer
            .offset((0, printer.size.y - cmdline_height - bottom_height))
            .cropped((printer.size.x, bottom_height));
        match &self.now_playing {
            Some(bar) if self.now_playing_shown(printer.size) => {
                // The cover screen already shows the cover, big.
                bar.set_show_cover(self.focus.as_deref() != Some("cover"));
                bar.draw(&bottom);
            }
            bar => {
                if let Some(bar) = bar {
                    bar.hide();
                }
                self.statusbar.draw(&bottom);
            }
        }

        if let Ok(Some(r)) = result {
            printer.print_hline((0, printer.size.y - cmdline_height), printer.size.x, " ");
            printer.print((0, printer.size.y - cmdline_height), &r);
        } else if let Err(e) = result {
            let style = ColorStyle::new(
                ColorType::Color(*self.theme.palette.custom("error").unwrap()),
                ColorType::Color(*self.theme.palette.custom("error_bg").unwrap()),
            );

            printer.with_color(style, |printer| {
                printer.print_hline((0, printer.size.y - cmdline_height), printer.size.x, " ");
                printer.print((0, printer.size.y - cmdline_height), &format!("ERROR: {e}"));
            });
        }

        if cmdline_visible {
            let printer = &printer.offset((0, printer.size.y - 1));
            self.cmdline.draw(printer);
        }
    }

    fn layout(&mut self, size: Vec2) {
        self.last_size = size;

        self.statusbar.layout(Vec2::new(size.x, 2));

        self.cmdline.layout(Vec2::new(size.x, 1));

        let bottom_height = self.bottom_height(size);
        if let Some(bar) = self.now_playing.as_mut() {
            bar.layout(Vec2::new(size.x, bottom_height));
        }

        let panel_width = self.queue_panel_width(size.x);
        let content_width = self.content_width(size.x);
        if let Some(panel) = self.queue_panel.as_mut() {
            panel.layout(Vec2::new(panel_width, size.y - bottom_height));
        }

        if let Some(view) = self.get_current_view_mut() {
            view.layout(Vec2::new(content_width, size.y - 1 - bottom_height));
        }
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        Vec2::new(constraint.x, constraint.y)
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        match event {
            Event::Key(Key::Esc) if self.cmdline_focus => {
                self.clear_cmdline();
                EventResult::consumed()
            }
            _ if self.cmdline_focus => self.command_line_handle_event(event),
            Event::Char(character)
                if !self.cmdline_focus
                    && (character
                        == self
                            .configuration
                            .values()
                            .command_key
                            .unwrap_or(config::DEFAULT_COMMAND_KEY)
                        || character == '/') =>
            {
                let result = self
                    .get_current_view_mut()
                    .map(|view| view.on_event(event))
                    .unwrap_or(EventResult::Ignored);

                if let EventResult::Ignored = result {
                    let command_key = self
                        .configuration
                        .values()
                        .command_key
                        .unwrap_or(config::DEFAULT_COMMAND_KEY);

                    if character == command_key {
                        self.enable_cmdline(command_key);
                        EventResult::consumed()
                    } else if character == '/' {
                        self.enable_jump();
                        EventResult::consumed()
                    } else {
                        EventResult::Ignored
                    }
                } else {
                    result
                }
            }
            Event::Mouse {
                position,
                event: mouse_event,
                ..
            } => {
                // Clicks and scrolling on the queue panel.
                let panel_width = self.queue_panel_width(self.last_size.x);
                let panel_x = self.last_size.x.saturating_sub(panel_width);
                let main_rows = self
                    .last_size
                    .y
                    .saturating_sub(self.bottom_height(self.last_size) + self.cmdline_height());
                if panel_width > 0
                    && position.x >= panel_x
                    && position.y < main_rows
                    && let Some(panel) = self.queue_panel.as_mut()
                {
                    return panel.on_event(event.relativized(Vec2::new(panel_x, 0)));
                }

                // Handle mouse events in the command/jump area.
                if position.y == 0 {
                    if mouse_event == MouseEvent::Press(MouseButton::Left)
                        && !self.is_current_stack_empty()
                        && position.x
                            < self
                                .get_current_screen()
                                .map(|screen| screen.title())
                                .unwrap_or_default()
                                .len()
                                + 3
                    {
                        self.pop_view();
                    }
                    return EventResult::consumed();
                }

                let result = self.get_result();

                let cmdline_visible = !self.cmdline.get_content().is_empty();
                let mut cmdline_height = usize::from(cmdline_visible);
                if result.as_ref().map(Option::is_some).unwrap_or(true) {
                    cmdline_height += 1;
                }

                let bottom_height = self.bottom_height(self.last_size);
                let bottom_y = self
                    .last_size
                    .y
                    .saturating_sub(bottom_height + cmdline_height);
                if position.y >= bottom_y && position.y < self.last_size.y - cmdline_height {
                    let event = event.relativized(Vec2::new(0, bottom_y));
                    if self.now_playing_shown(self.last_size)
                        && let Some(bar) = self.now_playing.as_mut()
                    {
                        bar.on_event(event);
                    } else {
                        self.statusbar.on_event(event);
                    }
                    return EventResult::consumed();
                }

                if let Some(view) = self.get_current_view_mut() {
                    view.on_event(event.relativized((0, 1)))
                } else {
                    EventResult::Ignored
                }
            }
            _ => {
                if let Some(view) = self.get_current_view_mut() {
                    view.on_event(event.relativized((0, 1)))
                } else {
                    EventResult::Ignored
                }
            }
        }
    }

    fn call_on_any(&mut self, s: &Selector, c: AnyCb<'_>) {
        if let Some(view) = self.get_current_view_mut() {
            view.call_on_any(s, c);
        }
    }

    fn take_focus(&mut self, source: Direction) -> Result<EventResult, CannotFocus> {
        if self.cmdline_focus {
            return self.cmdline.take_focus(source);
        }

        if let Some(view) = self.get_current_view_mut() {
            view.take_focus(source)
        } else {
            Err(CannotFocus)
        }
    }
}

impl ViewExt for Layout {
    fn on_command(&mut self, s: &mut Cursive, cmd: &Command) -> Result<CommandResult, String> {
        match cmd {
            Command::Focus(view) => {
                // Clear search results and return to search bar
                // If trying to focus search screen while already on it
                let search_view_name = "search";
                if view == search_view_name
                    && self.focus == Some(search_view_name.into())
                    && let Some(stack) = self.stack.get_mut(search_view_name)
                {
                    stack.clear();
                }

                if self.screens.keys().any(|k| k == view) {
                    self.set_screen(view.clone());
                    let screen = self.screens.get_mut(view).unwrap();
                    screen.on_command(s, cmd)?;
                }

                Ok(CommandResult::Consumed(None))
            }
            Command::Back => {
                self.back();
                Ok(CommandResult::Consumed(None))
            }
            _ => {
                if let Some(view) = self.get_current_view_mut() {
                    view.on_command(s, cmd)
                } else {
                    Ok(CommandResult::Ignored)
                }
            }
        }
    }
}

/// Cut `text` to at most `width` columns, ending with "…" if it was cut.
fn truncate_to_width(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// Completes the `input` using words from the `dictionary`.
///
/// # Returns
///
/// If no words match, `input` is returned unchanged. \
/// If a single word matches, that word is returned. \
/// If multiple words match, their common prefix is returned.
fn word_completion(input: &str, dictionary: &[&str]) -> String {
    let matching_words: Vec<_> = dictionary
        .iter()
        .filter(|word| word.starts_with(input))
        .collect();
    let Some(first_word) = matching_words.first() else {
        return input.into();
    };

    let mut prefix: Vec<char> = first_word.chars().collect();

    for word in matching_words {
        let common_len = prefix
            .iter()
            .copied()
            .zip(word.chars())
            .take_while(|(a, b)| a == b)
            .count();

        prefix.truncate(common_len);

        if prefix.is_empty() {
            break;
        }
    }

    prefix.into_iter().collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DICTIONARY: &[&str] = &["test1", "test2", "tset"];

    #[test]
    fn test_word_completion() {
        // One case from the dictionary matches the input.
        assert_eq!(word_completion("ts", TEST_DICTIONARY), "tset");

        // No cases from the dictionary match the input.
        assert_eq!(word_completion("Luna", TEST_DICTIONARY), "Luna");

        // Multiple cases from the dictionary with common prefix match the input.
        assert_eq!(word_completion("te", TEST_DICTIONARY), "test");

        // Multiple cases from the dictionary without common prefix match the input.
        assert_eq!(word_completion("t", TEST_DICTIONARY), "t");
    }
}
