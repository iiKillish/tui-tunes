//! The settings (`,`), over the page like the key list, in sections (General, Playback,
//! Appearance) shown one at a time: switches, choices and colors, each changed in place. Changes
//! are written to config.toml (keeping its comments) and take effect at once, except the few
//! marked as needing a restart. Keys are changed in the key list (`?`).

use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use strum::{IntoEnumIterator, VariantNames};

use crate::command::{Command, MoveAmount, MoveMode, ScreenId};
use crate::config::{ConfigTheme, ConfigValues, LibraryTab};
use crate::theme::parse_color;
use crate::tui::page::{Ctx, Outcome, Overlay};
use crate::tui::pointer;
use crate::tui::style::{SELECTION_MARKER, Styles, bold, fill, put, scroll_to, width};

/// A setting that's on or off.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Switch {
    LibraryTab(usize),
    PickUp,
    HideNames,
    Autoplay,
    Notify,
    Gapless,
    Volnorm,
    AudioCache,
    NerdFont,
}

/// A setting with a few values to pick from.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Choice {
    InitialScreen,
    Bitrate,
    VolumeCurve,
    Theme,
    ImageProtocol,
    QueueClick,
    TrackClick,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Title(&'static str),
    Blank,
    Switch(Switch),
    Choice(Choice),
    /// A color, by index into [COLORS].
    Color(usize),
}

impl Row {
    fn is_setting(self) -> bool {
        matches!(self, Self::Switch(_) | Self::Choice(_) | Self::Color(_))
    }
}

const SECTIONS: [&str; 3] = ["General", "Playback", "Appearance"];

/// The theme's colors: the keys in `[theme]` a row sets, and its name. The accent sets all the
/// colors that are the accent by default.
const COLORS: &[(&[&str], &str)] = &[
    (
        &["title", "playing", "highlight_bg", "statusbar_progress"],
        "Accent",
    ),
    (&["background"], "Background"),
    (&["primary"], "Text"),
    (&["secondary"], "Dim text"),
    (&["title"], "Titles"),
    (&["playing"], "Playing song"),
    (&["playing_bg"], "Playing song background"),
    (&["highlight"], "Text on buttons"),
    (&["highlight_bg"], "Buttons"),
    (&["highlight_inactive_bg"], "Selection without focus"),
    (&["statusbar_bg"], "Player and menus"),
    (&["statusbar_progress"], "Progress bar"),
    (&["statusbar_progress_bg"], "Progress bar track"),
    (&["error"], "Errors"),
    (&["error_bg"], "Error background"),
    (&["cmdline"], "Command line"),
    (&["cmdline_bg"], "Command line background"),
];

/// The colors ←/→ go through, by name.
const NAMED_COLORS: &[&str] = &[
    "default",
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "light black",
    "light red",
    "light green",
    "light yellow",
    "light blue",
    "light magenta",
    "light cyan",
    "light white",
];

/// Themes to pick from: name, then background, text, dim text and accent. The first is the
/// terminal's own colors (no `[theme]`).
const THEMES: &[(&str, [&str; 4])] = &[
    ("Terminal", ["", "", "", ""]),
    (
        "Catppuccin Mocha",
        ["#1e1e2e", "#cdd6f4", "#6c7086", "#cba6f7"],
    ),
    (
        "Catppuccin Latte",
        ["#eff1f5", "#4c4f69", "#8c8fa1", "#8839ef"],
    ),
    ("Gruvbox", ["#282828", "#ebdbb2", "#928374", "#fabd2f"]),
    ("Nord", ["#2e3440", "#d8dee9", "#616e88", "#88c0d0"]),
    ("Dracula", ["#282a36", "#f8f8f2", "#6272a4", "#bd93f9"]),
    ("Tokyo Night", ["#1a1b26", "#c0caf5", "#565f89", "#7aa2f7"]),
    ("Rosé Pine", ["#191724", "#e0def4", "#6e6a86", "#ebbcba"]),
];

/// The keys a theme from [THEMES] sets, in the order of its colors (the accent last).
const THEME_KEYS: [&str; 3] = ["background", "primary", "secondary"];

pub struct Settings {
    section: usize,
    rows: Vec<Row>,
    selected: usize,
    first: usize,
    /// The color being typed (row index into [COLORS], text), if any.
    editing: Option<(usize, String)>,
    /// Where the rows, the section tabs and the whole panel were drawn, for the mouse.
    list: Rect,
    tabs: Vec<Rect>,
    panel: Rect,
}

/// The library's tabs in their default order, with their names.
fn library_tabs() -> Vec<(LibraryTab, &'static str)> {
    LibraryTab::iter()
        .map(|tab| {
            let name = match tab {
                LibraryTab::Tracks => "Liked songs",
                LibraryTab::Albums => "Albums",
                LibraryTab::Artists => "Artists",
                LibraryTab::Playlists => "Playlists",
                LibraryTab::Podcasts => "Podcasts",
                LibraryTab::Browse => "Browse",
                LibraryTab::Local => "Local files",
            };
            (tab, name)
        })
        .collect()
}

/// The library tabs shown.
fn shown_tabs(values: &ConfigValues) -> Vec<LibraryTab> {
    values
        .library_tabs
        .clone()
        .unwrap_or_else(|| LibraryTab::iter().collect())
}

/// The rows of section `section`.
fn section_rows(section: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    match section {
        0 => {
            rows.push(Row::Choice(Choice::InitialScreen));
            rows.extend([Row::Blank, Row::Title("Library tabs")]);
            rows.extend((0..library_tabs().len()).map(|i| Row::Switch(Switch::LibraryTab(i))));
            rows.extend([
                Row::Blank,
                Row::Title("Home and playlists"),
                Row::Switch(Switch::PickUp),
                Row::Switch(Switch::HideNames),
            ]);
        }
        1 => rows.extend([
            Row::Switch(Switch::Autoplay),
            Row::Switch(Switch::Notify),
            Row::Choice(Choice::QueueClick),
            Row::Choice(Choice::TrackClick),
            Row::Blank,
            Row::Title("Sound"),
            Row::Choice(Choice::Bitrate),
            Row::Switch(Switch::Gapless),
            Row::Switch(Switch::Volnorm),
            Row::Choice(Choice::VolumeCurve),
            Row::Switch(Switch::AudioCache),
        ]),
        _ => {
            rows.extend([
                Row::Choice(Choice::Theme),
                Row::Color(0),
                Row::Switch(Switch::NerdFont),
                Row::Choice(Choice::ImageProtocol),
                Row::Blank,
                Row::Title("Colors"),
            ]);
            rows.extend((1..COLORS.len()).map(Row::Color));
        }
    }
    rows
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    pub fn new() -> Self {
        let mut page = Self {
            section: 0,
            rows: Vec::new(),
            selected: 0,
            first: 0,
            editing: None,
            list: Rect::default(),
            tabs: Vec::new(),
            panel: Rect::default(),
        };
        page.show_section(0);
        page
    }

    fn show_section(&mut self, section: usize) {
        self.section = section % SECTIONS.len();
        self.rows = section_rows(self.section);
        self.first = 0;
        self.selected = self.next_row(0, true).unwrap_or(0);
        self.editing = None;
    }

    /// The first setting from `from` on, going down (or up).
    fn next_row(&self, from: usize, down: bool) -> Option<usize> {
        let setting = |i: &usize| self.rows.get(*i).is_some_and(|r| r.is_setting());
        if down {
            (from..self.rows.len()).find(setting)
        } else {
            (0..=from.min(self.rows.len().saturating_sub(1)))
                .rev()
                .find(setting)
        }
    }

    fn step(&mut self, down: bool, by: usize) {
        for _ in 0..by {
            let next = if down {
                self.next_row(self.selected + 1, true)
            } else {
                self.selected
                    .checked_sub(1)
                    .and_then(|i| self.next_row(i, false))
            };
            match next {
                Some(next) => self.selected = next,
                None => break,
            }
        }
    }

    /// Change the selected setting: turn a switch over, or go to the next (or previous) value.
    fn change(&mut self, forward: bool, ctx: &Ctx) -> Outcome {
        match self.rows.get(self.selected).copied() {
            Some(Row::Switch(switch)) => toggle(switch, ctx),
            Some(Row::Choice(choice)) => {
                let options = options(choice);
                let len = options.len();
                let next = match current(choice, &ctx.config.values()) {
                    Some(i) if forward => (i + 1) % len,
                    Some(i) => (i + len - 1) % len,
                    None => 0,
                };
                saved(ctx.config.edit(|doc| set_choice(doc, choice, next)), None)
            }
            Some(Row::Color(index)) => {
                let now = color_value(&ctx.config.values().theme, index);
                let at = NAMED_COLORS
                    .iter()
                    .position(|c| Some(*c) == now.as_deref())
                    .unwrap_or(0);
                let len = NAMED_COLORS.len();
                let next = if forward {
                    (at + 1) % len
                } else {
                    (at + len - 1) % len
                };
                set_color(index, Some(NAMED_COLORS[next]), ctx)
            }
            _ => Outcome::Consumed,
        }
    }
}

fn switch_state(switch: Switch, values: &ConfigValues) -> bool {
    match switch {
        Switch::LibraryTab(i) => shown_tabs(values).contains(&library_tabs()[i].0),
        Switch::PickUp => values.pick_up_where_you_left_off != Some(false),
        Switch::HideNames => values.hide_display_names == Some(true),
        Switch::Autoplay => values.autoplay != Some(false),
        Switch::Notify => values.notify == Some(true),
        Switch::Gapless => values.gapless != Some(false),
        Switch::Volnorm => values.volnorm == Some(true),
        Switch::AudioCache => values.audio_cache != Some(false),
        Switch::NerdFont => values.use_nerdfont == Some(true),
    }
}

/// The switch's label, and whether a change needs a restart.
fn switch_label(switch: Switch) -> (&'static str, bool) {
    match switch {
        Switch::LibraryTab(i) => (library_tabs()[i].1, false),
        Switch::PickUp => ("Pick up where you left off, on Home", false),
        Switch::HideNames => ("Hide the names of playlists' owners", false),
        Switch::Autoplay => ("Autoplay similar songs when the queue ends", false),
        Switch::Notify => ("Notify when a song starts", false),
        Switch::Gapless => ("Gapless playback", true),
        Switch::Volnorm => ("Even out loudness between songs", true),
        Switch::AudioCache => ("Keep played songs on disk", true),
        Switch::NerdFont => ("Nerd Font icons", false),
    }
}

/// The key in the configuration file for a switch other than the library's tabs.
fn switch_key(switch: Switch) -> &'static str {
    match switch {
        Switch::LibraryTab(_) => "library_tabs",
        Switch::PickUp => "pick_up_where_you_left_off",
        Switch::HideNames => "hide_display_names",
        Switch::Autoplay => "autoplay",
        Switch::Notify => "notify",
        Switch::Gapless => "gapless",
        Switch::Volnorm => "volnorm",
        Switch::AudioCache => "audio_cache",
        Switch::NerdFont => "use_nerdfont",
    }
}

/// Turn `switch` on or off in the configuration file.
fn toggle(switch: Switch, ctx: &Ctx) -> Outcome {
    let on = switch_state(switch, &ctx.config.values());
    let result = match switch {
        Switch::LibraryTab(i) => {
            let tab = library_tabs()[i].0;
            let shown = shown_tabs(&ctx.config.values());
            // In the default order, with the tab added or left out.
            let tabs: Vec<LibraryTab> = LibraryTab::iter()
                .filter(|t| if *t == tab { !on } else { shown.contains(t) })
                .collect();
            if tabs.is_empty() {
                return Outcome::Error("The library needs at least one tab".into());
            }
            ctx.config.edit(|doc| {
                let mut names = toml_edit::Array::new();
                for tab in &tabs {
                    names.push(tab_name(*tab));
                }
                doc["library_tabs"] = toml_edit::value(names);
            })
        }
        switch => ctx.config.edit(|doc| {
            doc[switch_key(switch)] = toml_edit::value(!on);
        }),
    };
    saved(result, restart_note(switch_label(switch).1))
}

fn restart_note(restart: bool) -> Option<String> {
    restart.then(|| "Saved: restart ttunes for it to take effect".into())
}

/// A choice's label, and whether a change needs a restart.
fn choice_label(choice: Choice) -> (&'static str, bool) {
    match choice {
        Choice::InitialScreen => ("Start on", false),
        Choice::Bitrate => ("Quality", true),
        Choice::VolumeCurve => ("Volume steps", true),
        Choice::Theme => ("Theme", false),
        Choice::ImageProtocol => ("Covers", true),
        Choice::QueueClick => ("Clicking a song in Up next", false),
        Choice::TrackClick => ("Clicking a song elsewhere", false),
    }
}

/// What a choice can be: the value written (empty to leave the key out, for the default) and
/// its name.
fn options(choice: Choice) -> Vec<(&'static str, &'static str)> {
    match choice {
        Choice::InitialScreen => ScreenId::VARIANTS
            .iter()
            .map(|v| {
                let id: ScreenId = v.parse().expect("a screen");
                (*v, id.label())
            })
            .collect(),
        Choice::Bitrate => vec![
            ("96", "Low (96 kbit/s)"),
            ("160", "Normal (160 kbit/s)"),
            ("", "High (320 kbit/s)"),
        ],
        Choice::VolumeCurve => vec![
            ("", "Like the system's (cubic)"),
            ("log", "Quiet sooner (log)"),
            ("linear", "Even (linear)"),
        ],
        Choice::Theme => THEMES.iter().map(|(name, _)| ("", *name)).collect(),
        Choice::ImageProtocol => vec![
            ("", "Judged from the terminal"),
            ("kitty", "Kitty graphics"),
            ("sixel", "Sixel"),
            ("iterm2", "iTerm2 images"),
            ("none", "Off"),
        ],
        Choice::QueueClick => vec![("", "Plays it"), ("menu", "Opens its menu")],
        Choice::TrackClick => vec![
            ("", "Selects it, then plays it"),
            ("menu", "Opens its menu"),
        ],
    }
}

/// The index of the choice's value in [options], if it's one of them (a theme edited by hand
/// isn't).
fn current(choice: Choice, values: &ConfigValues) -> Option<usize> {
    let value = match choice {
        Choice::InitialScreen => values.initial_screen.clone().unwrap_or("home".into()),
        Choice::Bitrate => match values.bitrate {
            None | Some(320) => String::new(),
            Some(rate) => rate.to_string(),
        },
        Choice::VolumeCurve => match values.volume_curve.as_deref() {
            None | Some("cubic") => String::new(),
            Some(curve) => curve.to_string(),
        },
        Choice::ImageProtocol => match values.image_protocol.as_deref() {
            None | Some("auto") => String::new(),
            Some(protocol) => protocol.to_lowercase(),
        },
        Choice::QueueClick => match values.queue_click.as_deref() {
            None | Some("play") => String::new(),
            Some(action) => action.to_lowercase(),
        },
        Choice::TrackClick => match values.track_click.as_deref() {
            None | Some("play") => String::new(),
            Some(action) => action.to_lowercase(),
        },
        Choice::Theme => return current_theme(&values.theme),
    };
    options(choice).iter().position(|(v, _)| *v == value)
}

/// Which of [THEMES] `theme` is, if one: its background, text, dim text and accent are the
/// theme's, and nothing else is set but what the accent sets.
fn current_theme(theme: &Option<ConfigTheme>) -> Option<usize> {
    let set: Vec<(&str, String)> = COLORS[1..]
        .iter()
        .filter_map(|(keys, _)| Some((keys[0], color_of(theme, keys[0])?.to_lowercase())))
        .collect();
    THEMES.iter().position(|(_, colors)| {
        let expected = |key: &str| match THEME_KEYS.iter().position(|k| *k == key) {
            Some(i) => colors[i],
            None if COLORS[0].0.contains(&key) => colors[3],
            None => "",
        };
        let wanted = COLORS[1..]
            .iter()
            .filter(|(keys, _)| !expected(keys[0]).is_empty())
            .count();
        set.len() == wanted && set.iter().all(|(key, value)| expected(key) == value)
    })
}

fn set_choice(doc: &mut toml_edit::DocumentMut, choice: Choice, index: usize) {
    let (value, _) = options(choice)[index];
    if choice == Choice::Theme {
        let table = theme_table(doc);
        table.clear();
        let colors = THEMES[index].1;
        for (key, color) in THEME_KEYS.iter().zip(colors) {
            if !color.is_empty() {
                table[key] = toml_edit::value(color);
            }
        }
        for key in COLORS[0].0 {
            if !colors[3].is_empty() {
                table[key] = toml_edit::value(colors[3]);
            }
        }
        return;
    }
    let key = match choice {
        Choice::InitialScreen => "initial_screen",
        Choice::Bitrate => "bitrate",
        Choice::VolumeCurve => "volume_curve",
        Choice::ImageProtocol => "image_protocol",
        Choice::QueueClick => "queue_click",
        Choice::TrackClick => "track_click",
        Choice::Theme => unreachable!(),
    };
    if value.is_empty() {
        doc.remove(key);
    } else if let Ok(number) = value.parse::<i64>() {
        doc[key] = toml_edit::value(number);
    } else {
        doc[key] = toml_edit::value(value);
    }
}

/// The value of the theme's color `key`, as written.
fn color_of(theme: &Option<ConfigTheme>, key: &str) -> Option<String> {
    let theme = theme.as_ref()?;
    match key {
        "background" => theme.background.clone(),
        "primary" => theme.primary.clone(),
        "secondary" => theme.secondary.clone(),
        "title" => theme.title.clone(),
        "playing" => theme.playing.clone(),
        "playing_bg" => theme.playing_bg.clone(),
        "highlight" => theme.highlight.clone(),
        "highlight_bg" => theme.highlight_bg.clone(),
        "highlight_inactive_bg" => theme.highlight_inactive_bg.clone(),
        "error" => theme.error.clone(),
        "error_bg" => theme.error_bg.clone(),
        "statusbar_progress" => theme.statusbar_progress.clone(),
        "statusbar_progress_bg" => theme.statusbar_progress_bg.clone(),
        "statusbar_bg" => theme.statusbar_bg.clone(),
        "cmdline" => theme.cmdline.clone(),
        "cmdline_bg" => theme.cmdline_bg.clone(),
        _ => None,
    }
}

/// The value of color row `index` (for the accent, the titles').
fn color_value(theme: &Option<ConfigTheme>, index: usize) -> Option<String> {
    color_of(theme, COLORS[index].0[0])
}

/// The color the row `index` is drawn in now.
fn palette_color(ctx: &Ctx, index: usize) -> Color {
    let p = &ctx.palette;
    match COLORS[index].0[0] {
        "background" => p.background,
        "primary" => p.primary,
        "secondary" => p.secondary,
        "title" => p.title,
        "playing" => p.playing,
        "playing_bg" => p.playing_bg,
        "highlight" => p.highlight,
        "highlight_bg" => p.highlight_bg,
        "highlight_inactive_bg" => p.highlight_inactive_bg,
        "error" => p.error,
        "error_bg" => p.error_bg,
        "statusbar_progress" => p.statusbar_progress,
        "statusbar_progress_bg" => p.statusbar_progress_bg,
        "statusbar_bg" => p.statusbar_bg,
        "cmdline" => p.cmdline,
        _ => p.cmdline_bg,
    }
}

/// Set color row `index` to `value`, or back to the default.
fn set_color(index: usize, value: Option<&str>, ctx: &Ctx) -> Outcome {
    if let Some(value) = value
        && parse_color(value).is_none()
    {
        return Outcome::Error(format!(
            "\"{value}\" isn't a color: use #rrggbb, #rgb or a name like \"light blue\""
        ));
    }
    let result = ctx.config.edit(|doc| {
        let table = theme_table(doc);
        for key in COLORS[index].0 {
            match value {
                Some(value) => table[key] = toml_edit::value(value),
                None => {
                    table.remove(key);
                }
            }
        }
    });
    saved(result, None)
}

/// After a change to the configuration file: have the app use the new settings (and say so if
/// it waits for a restart), or say what went wrong.
fn saved(result: Result<(), Box<dyn std::error::Error>>, message: Option<String>) -> Outcome {
    match result {
        Ok(()) => Outcome::SettingsChanged(message),
        Err(e) => Outcome::Error(format!("Couldn't save the settings: {e}")),
    }
}

/// The `[theme]` table of the configuration, made if it's missing.
fn theme_table(doc: &mut toml_edit::DocumentMut) -> &mut toml_edit::Table {
    if !doc.contains_table("theme") {
        doc["theme"] = toml_edit::table();
    }
    doc["theme"].as_table_mut().expect("just made")
}

/// The name of `tab` in `library_tabs`.
fn tab_name(tab: LibraryTab) -> &'static str {
    match tab {
        LibraryTab::Tracks => "tracks",
        LibraryTab::Albums => "albums",
        LibraryTab::Artists => "artists",
        LibraryTab::Playlists => "playlists",
        LibraryTab::Podcasts => "podcasts",
        LibraryTab::Browse => "browse",
        LibraryTab::Local => "local",
    }
}

/// Width of the labels' column for choices and colors.
const LABEL_WIDTH: usize = 26;

impl Overlay for Settings {
    fn draw(&mut self, frame: &mut Frame, screen: Rect, ctx: &Ctx) -> Rect {
        let styles = Styles(&ctx.palette);
        let w = screen
            .width
            .saturating_sub(4)
            .clamp(10, 80)
            .min(screen.width);
        let h = screen.height.saturating_sub(2).max(3).min(screen.height);
        let panel = Rect::new(
            screen.x + screen.width.saturating_sub(w) / 2,
            screen.y + screen.height.saturating_sub(h) / 2,
            w,
            h,
        );
        self.panel = panel;
        frame.render_widget(Clear, panel);
        let background = ctx.palette.statusbar_bg;
        let border = styles.accent().bg(background);
        let dim = styles.panel().fg(ctx.palette.secondary);
        let selected_row = self.rows.get(self.selected).copied();
        let tip = match (&self.editing, selected_row) {
            (Some(_), _) => " Type a color (#rrggbb or a name) · Enter saves · Esc cancels ",
            (None, Some(Row::Color(_))) => {
                " Tab sections · ←→ pick · Enter types one · d default · Esc closes "
            }
            (None, Some(Row::Choice(_))) => " Tab sections · ←→ or Enter change · Esc closes ",
            _ => " Tab sections · Enter changes · Esc closes ",
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border)
            .title(" Settings ")
            .title_style(bold(border))
            .title_bottom(Line::styled(tip, dim))
            .style(styles.panel());
        let area = block.inner(panel);
        frame.render_widget(block, panel);
        let buf = frame.buffer_mut();
        let room = usize::from(area.width.saturating_sub(4));

        // The sections, the current one lit, and where the settings go.
        self.tabs.clear();
        let mut x = area.x + 1;
        for (i, title) in SECTIONS.iter().enumerate() {
            let label = format!(" {title} ");
            let style = if i == self.section {
                styles.button()
            } else {
                styles.panel()
            };
            let left = usize::from(area.right().saturating_sub(x + 1));
            let written = put(buf, x, area.y, &label, left, style) as u16;
            let tab = Rect::new(x, area.y, written, 1);
            pointer::clickable(tab);
            self.tabs.push(tab);
            x += written + 1;
        }
        let note = "saved to config.toml";
        let nx = area.right().saturating_sub(width(note) as u16 + 2);
        if nx > x + 2 {
            put(buf, nx, area.y, note, usize::MAX, dim);
        }
        let list = Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        );
        self.list = list;

        let rows = usize::from(list.height);
        self.first = scroll_to(self.first, self.selected, rows);
        // Keep the titles above the selected row in view, up to the top for the first row.
        let mut top = self.selected;
        while top > 0 && !self.rows.get(top - 1).is_some_and(|r| r.is_setting()) {
            top -= 1;
        }
        if top < self.first && self.selected - top < rows {
            self.first = top;
        }
        let values = ctx.config.values().clone();

        for (i, row) in self.rows.iter().enumerate().skip(self.first).take(rows) {
            let y = list.y + (i - self.first) as u16;
            let line = Rect::new(list.x, y, list.width, 1);
            let selected = i == self.selected;
            let style = if selected {
                styles.selected()
            } else {
                styles.panel()
            };
            match row {
                Row::Blank => continue,
                Row::Title(title) => {
                    put(buf, area.x + 2, y, title, room, bold(border));
                    continue;
                }
                _ => {
                    fill(buf, line, style);
                    pointer::clickable(line);
                    if selected {
                        put(buf, area.x, y, SELECTION_MARKER, 1, styles.marker());
                    }
                }
            }
            let value_x = area.x + 2 + LABEL_WIDTH as u16;
            let value_room = usize::from(area.right().saturating_sub(value_x + 1));
            let value_style = bold(style.fg(ctx.palette.title));
            let restart = |buf: &mut ratatui::buffer::Buffer, x: u16| {
                put(
                    buf,
                    x + 2,
                    y,
                    "after restart",
                    usize::from(area.right().saturating_sub(x + 3)),
                    style.fg(ctx.palette.secondary),
                );
            };
            match row {
                Row::Switch(switch) => {
                    let on = switch_state(*switch, &values);
                    let (mark, mark_style) = if on {
                        ("[✓]", value_style)
                    } else {
                        ("[ ]", style.fg(ctx.palette.secondary))
                    };
                    put(buf, area.x + 2, y, mark, 3, mark_style);
                    let (label, needs_restart) = switch_label(*switch);
                    let used = put(buf, area.x + 6, y, label, room, style);
                    if needs_restart {
                        restart(buf, area.x + 6 + used as u16);
                    }
                }
                Row::Choice(choice) => {
                    let (label, needs_restart) = choice_label(*choice);
                    put(buf, area.x + 2, y, label, LABEL_WIDTH, style);
                    let value = match current(*choice, &values) {
                        Some(i) => options(*choice)[i].1,
                        None => "Custom",
                    };
                    let text = format!("‹ {value} ›");
                    let used = put(buf, value_x, y, &text, value_room, value_style);
                    if needs_restart {
                        restart(buf, value_x + used as u16);
                    }
                }
                Row::Color(index) => {
                    put(buf, area.x + 2, y, COLORS[*index].1, LABEL_WIDTH, style);
                    // A swatch in the color as it is, then its value.
                    put(
                        buf,
                        value_x,
                        y,
                        "██",
                        2,
                        style.fg(palette_color(ctx, *index)),
                    );
                    let text = match &self.editing {
                        Some((editing, text)) if editing == index => format!("{text}▏"),
                        _ => color_value(&values.theme, *index).unwrap_or_else(|| "default".into()),
                    };
                    put(
                        buf,
                        value_x + 3,
                        y,
                        &text,
                        value_room.saturating_sub(3),
                        value_style,
                    );
                }
                _ => {}
            }
        }
        panel
    }

    fn command(&mut self, cmd: &Command, ctx: &Ctx) -> Outcome {
        let page = usize::from(self.list.height.max(1));
        match cmd {
            Command::Back | Command::Settings => return Outcome::Close,
            Command::Move(MoveMode::Up, amount) => self.step(false, steps(amount, page)),
            Command::Move(MoveMode::Down, amount) => self.step(true, steps(amount, page)),
            Command::Move(MoveMode::Left, _) => return self.change(false, ctx),
            Command::Move(MoveMode::Right, _) => return self.change(true, ctx),
            Command::Tab(next) | Command::Screen(next) => {
                let len = SECTIONS.len();
                self.show_section(if *next {
                    self.section + 1
                } else {
                    self.section + len - 1
                });
            }
            Command::Play | Command::Open(_) => {
                if let Some(Row::Color(index)) = self.rows.get(self.selected).copied() {
                    let now = color_value(&ctx.config.values().theme, index).unwrap_or_default();
                    self.editing = Some((index, now));
                    return Outcome::Consumed;
                }
                return self.change(true, ctx);
            }
            Command::Delete => {
                if let Some(Row::Color(index)) = self.rows.get(self.selected).copied() {
                    return set_color(index, None, ctx);
                }
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }

    /// While a color is typed, every key comes here; Tab always goes from section to section.
    fn captures_text(&self) -> bool {
        true
    }

    fn key(&mut self, event: KeyEvent, ctx: &Ctx) -> Outcome {
        let Some((index, text)) = self.editing.as_mut() else {
            return match event.code {
                KeyCode::Tab => self.command(&Command::Tab(true), ctx),
                KeyCode::BackTab => self.command(&Command::Tab(false), ctx),
                _ => Outcome::Ignored,
            };
        };
        match event.code {
            KeyCode::Esc => self.editing = None,
            KeyCode::Enter => {
                let (index, text) = (*index, text.trim().to_string());
                self.editing = None;
                let value = (!text.is_empty()).then_some(text.as_str());
                return set_color(index, value, ctx);
            }
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) if !event.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
            _ => {}
        }
        Outcome::Consumed
    }

    fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let inside = |area: Rect| area.contains((event.column, event.row).into());
        // A section's tab (not `if let` in a match guard, which older Rust doesn't have).
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some(section) = self.tabs.iter().position(|t| inside(*t))
        {
            self.show_section(section);
            return Outcome::Consumed;
        }
        match event.kind {
            MouseEventKind::Down(_) if !inside(self.panel) => return Outcome::Close,
            MouseEventKind::Down(MouseButton::Left) if inside(self.list) => {
                let row = self.first + usize::from(event.row - self.list.y);
                if self.rows.get(row).is_some_and(|r| r.is_setting()) {
                    self.selected = row;
                    return self.command(&Command::Play, ctx);
                }
            }
            MouseEventKind::ScrollDown => self.step(true, 3),
            MouseEventKind::ScrollUp => self.step(false, 3),
            _ => {}
        }
        Outcome::Consumed
    }
}

/// How many rows a move of `amount` goes, with `page` rows showing.
fn steps(amount: &MoveAmount, page: usize) -> usize {
    match amount {
        MoveAmount::Integer(n) => n.unsigned_abs() as usize,
        MoveAmount::Float(scale) => ((page as f32 * scale) as usize).max(1),
        MoveAmount::Extreme => usize::MAX / 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_themes_are_recognised() {
        assert_eq!(current_theme(&None), Some(0));
        for (i, _) in THEMES.iter().enumerate() {
            let mut doc = toml_edit::DocumentMut::new();
            set_choice(&mut doc, Choice::Theme, i);
            let values: ConfigValues = toml::from_str(&doc.to_string()).unwrap();
            assert_eq!(current_theme(&values.theme), Some(i), "{}", THEMES[i].0);
        }
        // Changed by hand: no longer one of them.
        let theme = ConfigTheme {
            background: Some("#000000".into()),
            ..Default::default()
        };
        assert_eq!(current_theme(&Some(theme)), None);
        for (_, colors) in THEMES {
            assert!(
                colors
                    .iter()
                    .all(|c| c.is_empty() || parse_color(c).is_some())
            );
        }
    }
}
