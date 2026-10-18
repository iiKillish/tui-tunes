//! The player: the current track's cover, title (with a like button), artist and album, playback
//! controls, progress and volume. All of it can be clicked.
//!
//! Drawn as a **hero** when there's room (design B + K3): the cover big and centered, the track
//! under it, then the progress bar, then shuffle at its left end, previous / play-pause / next in
//! the middle and repeat at its right end, then the volume. In small windows it's a one-line-high
//! **bar**: cover, track, controls over the progress bar, volume.

use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;

use crate::command::Command;
use crate::model::playable::Playable;
use crate::queue::{RepeatSetting, SleepTimer};
use crate::spotify::{PlayerEvent, VOLUME_PERCENT};
use crate::tui::hints;
#[cfg(feature = "cover")]
use crate::tui::icons::Icon;
use crate::tui::images;
#[cfg(feature = "cover")]
use crate::tui::images::Source;
use crate::tui::page::{Ctx, Outcome};
use crate::tui::pointer;
use crate::tui::style::{Styles, bold, duration, fill, put, width};

/// Rows the hero needs besides the cover: title, artist, album, gap, progress, times, controls
/// (two rows as images), gap, volume, margins.
const HERO_CHROME_ROWS: u16 = 12;
/// Fewest rows to draw the hero at all.
pub const HERO_MIN_ROWS: u16 = 14;
/// Rows of the bar.
pub const BAR_ROWS: u16 = 5;
const VOLUME_BAR_WIDTH: u16 = 12;
/// Narrowest track text beside the bar's controls; narrower bars put the title above them.
const MIN_BAR_TEXT_WIDTH: u16 = 16;
/// Smallest cover worth drawing above the controls; below this the hero puts it beside them.
const MIN_COVER_ROWS_ABOVE: u16 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Control {
    Shuffle,
    Previous,
    PlayPause,
    Next,
    Repeat,
}

/// Where the clickable parts were drawn last.
#[derive(Default)]
struct Hits {
    controls: Vec<(Rect, Control)>,
    like: Option<Rect>,
    progress: Option<Rect>,
    volume: Option<Rect>,
}

#[derive(Default)]
pub struct Player {
    hits: Hits,
    /// The volume knob is being dragged.
    dragging_volume: bool,
}

/// Whether the controls are drawn as (bigger) images.
fn image_controls() -> bool {
    cfg!(feature = "cover") && images::supported()
}

impl Player {
    pub fn new() -> Self {
        Self::default()
    }

    fn nerdfont(ctx: &Ctx) -> bool {
        ctx.config.values().use_nerdfont.unwrap_or(false)
    }

    fn playing(ctx: &Ctx) -> bool {
        matches!(ctx.spotify.get_current_status(), PlayerEvent::Playing(_))
    }

    fn control_active(control: Control, ctx: &Ctx) -> bool {
        match control {
            Control::Shuffle => ctx.queue.get_shuffle(),
            Control::Repeat => ctx.queue.get_repeat() != RepeatSetting::None,
            _ => false,
        }
    }

    #[cfg(feature = "cover")]
    fn control_image(control: Control, ctx: &Ctx) -> Icon {
        match control {
            Control::Shuffle => Icon::Shuffle,
            Control::Previous => Icon::Previous,
            Control::PlayPause if Self::playing(ctx) => Icon::Pause,
            Control::PlayPause => Icon::Play,
            Control::Next => Icon::Next,
            Control::Repeat if ctx.queue.get_repeat() == RepeatSetting::RepeatTrack => {
                Icon::RepeatOne
            }
            Control::Repeat => Icon::Repeat,
        }
    }

    fn control_text(control: Control, ctx: &Ctx) -> &'static str {
        let playing = Self::playing(ctx);
        let repeat_one = ctx.queue.get_repeat() == RepeatSetting::RepeatTrack;
        if Self::nerdfont(ctx) {
            match control {
                Control::Shuffle => "\u{f049d}",
                Control::Previous => "\u{f04ae}",
                Control::PlayPause if playing => "\u{f03e4}",
                Control::PlayPause => "\u{f040a}",
                Control::Next => "\u{f04ad}",
                Control::Repeat if repeat_one => "\u{f0458}",
                Control::Repeat => "\u{f0456}",
            }
        } else {
            match control {
                Control::Shuffle => "⇄",
                Control::Previous => "⏮",
                Control::PlayPause if playing => "⏸",
                Control::PlayPause => "▶",
                Control::Next => "⏭",
                Control::Repeat if repeat_one => "↻¹",
                Control::Repeat => "↻",
            }
        }
    }

    fn like_text(liked: bool, ctx: &Ctx) -> &'static str {
        match (Self::nerdfont(ctx), liked) {
            (true, true) => "\u{f02d1}",
            (true, false) => "\u{f02d5}",
            (false, true) => "♥",
            (false, false) => "♡",
        }
    }

    fn volume_text(ctx: &Ctx) -> &'static str {
        let volume = ctx.spotify.volume();
        if Self::nerdfont(ctx) {
            match volume {
                0 => "\u{f0581}",
                v if v < u16::MAX / 3 => "\u{f057f}",
                v if v < u16::MAX / 3 * 2 => "\u{f0580}",
                _ => "\u{f057e}",
            }
        } else if volume == 0 {
            "×"
        } else {
            "♪"
        }
    }

    /// Size of a control button: square images two rows tall, or a few cells of text.
    fn button_size() -> (u16, u16) {
        if image_controls() {
            (images::square_width(2), 2)
        } else {
            (3, 1)
        }
    }

    /// Draw one control at `area`; shuffle and repeat are in the accent when on.
    fn draw_control(&mut self, frame: &mut Frame, area: Rect, control: Control, ctx: &Ctx) {
        let styles = Styles(&ctx.palette);
        let active = Self::control_active(control, ctx);
        let color = match control {
            Control::Shuffle | Control::Repeat if active => ctx.palette.title,
            Control::Shuffle | Control::Repeat => ctx.palette.secondary,
            _ => ctx.palette.primary,
        };
        self.hits.controls.push((area, control));
        #[cfg(feature = "cover")]
        if image_controls() {
            images::request_image(
                Source::Icon(Self::control_image(control, ctx), ctx.palette.rgb(color)),
                area,
                0,
            );
            return;
        }
        let text = Self::control_text(control, ctx);
        let x = area.x + area.width.saturating_sub(width(text) as u16) / 2;
        put(
            frame.buffer_mut(),
            x,
            area.y,
            text,
            usize::from(area.width),
            bold(
                Style::new()
                    .fg(color)
                    .bg(styles.panel().bg.unwrap_or_default()),
            ),
        );
    }

    /// A horizontal bar filled to `fraction`.
    fn draw_bar(frame: &mut Frame, area: Rect, fraction: f64, ctx: &Ctx, background: Style) {
        let filled =
            ((fraction.clamp(0.0, 1.0) * f64::from(area.width)).round() as u16).min(area.width);
        let buf = frame.buffer_mut();
        let empty = background.fg(ctx.palette.statusbar_progress_bg);
        let full = background.fg(ctx.palette.statusbar_progress);
        for i in 0..area.width {
            let style = if i < filled { full } else { empty };
            buf.set_string(area.x + i, area.y, "━", style);
        }
    }

    /// The volume bar with a knob (●) at the current level, which can be dragged.
    fn draw_volume(frame: &mut Frame, area: Rect, fraction: f64, ctx: &Ctx, background: Style) {
        Self::draw_bar(frame, area, fraction, ctx, background);
        if area.width == 0 {
            return;
        }
        let filled = (fraction.clamp(0.0, 1.0) * f64::from(area.width)).round() as u16;
        let x = area.x + filled.saturating_sub(1).min(area.width - 1);
        put(
            frame.buffer_mut(),
            x,
            area.y,
            "●",
            1,
            background.fg(ctx.palette.primary),
        );
    }

    /// Whether a drag started on the player (the volume knob) is going on: it then gets every
    /// mouse event, wherever the pointer is.
    pub fn dragging(&self) -> bool {
        self.dragging_volume
    }

    /// Set the volume from a pointer at column `x`, for the volume bar starting at `bar_x`.
    fn set_volume_at(x: u16, bar_x: u16, ctx: &Ctx) {
        let fraction = if x < bar_x {
            0.0
        } else {
            (f64::from(x - bar_x + 1) / f64::from(VOLUME_BAR_WIDTH)).min(1.0)
        };
        ctx.spotify
            .set_volume((f64::from(u16::MAX) * fraction) as u16, true);
    }

    /// The text lines: title (with the like button), artists, album. Centered in the hero.
    fn draw_text(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        centered: bool,
        ctx: &Ctx,
        background: Style,
    ) {
        let styles = Styles(&ctx.palette);
        let track = ctx.queue.get_current();
        let (title, artists, album) = match &track {
            Some(t) => (
                Playable::format(t, "%title", &ctx.library),
                Playable::format(t, "%artists", &ctx.library),
                Playable::format(t, "%album", &ctx.library),
            ),
            None => ("Nothing playing".to_string(), String::new(), String::new()),
        };
        let w = usize::from(area.width);
        let liked = match &track {
            Some(t @ Playable::Track(_)) if *ctx.library.is_done.read().unwrap() && w > 6 => {
                Some(ctx.library.is_saved_track(t))
            }
            _ => None,
        };
        let title = crate::tui::style::truncate(&title, if liked.is_some() { w - 3 } else { w });
        let line_x = |text_width: usize| {
            if centered {
                area.x + (w.saturating_sub(text_width) / 2) as u16
            } else {
                area.x
            }
        };
        let buf = frame.buffer_mut();
        let title_width = width(&title) + if liked.is_some() { 3 } else { 0 };
        let x = line_x(title_width);
        put(
            buf,
            x,
            area.y,
            &title,
            w,
            bold(background.fg(ctx.palette.primary)),
        );
        self.hits.like = None;
        if let Some(liked) = liked {
            let like_x = x + width(&title) as u16 + 2;
            let color = if liked {
                ctx.palette.title
            } else {
                ctx.palette.secondary
            };
            put(
                buf,
                like_x,
                area.y,
                Self::like_text(liked, ctx),
                2,
                background.fg(color),
            );
            self.hits.like = Some(Rect::new(like_x, area.y, 2, 1));
        }
        if area.height < 2 {
            return;
        }
        let artists = crate::tui::style::truncate(&artists, w);
        let album = crate::tui::style::truncate(&album, w);
        put(
            buf,
            line_x(width(&artists)),
            area.y + 1,
            &artists,
            w,
            background.fg(ctx.palette.secondary),
        );
        if area.height < 3 {
            return;
        }
        put(
            buf,
            line_x(width(&album)),
            area.y + 2,
            &album,
            w,
            background.fg(styles.accent().fg.unwrap_or_default()),
        );
    }

    /// The count prefix being typed, the sleep timer, library loading and rate limits, top right.
    fn draw_status(frame: &mut Frame, area: Rect, count: usize, ctx: &Ctx, background: Style) {
        let mut status = Vec::new();
        if count > 0 {
            status.push(count.to_string());
        }
        match ctx.queue.sleep_timer() {
            Some(SleepTimer::EndOfTrack) => status.push("sleep after this track".to_string()),
            Some(SleepTimer::At(at)) => {
                let left = at.saturating_duration_since(std::time::Instant::now());
                // Round up, so it shows 0:00 only as it pauses.
                let ms = (left.as_millis() as u32).div_ceil(1000) * 1000;
                status.push(format!("sleep {}", duration(ms)));
            }
            None => {}
        }
        if !*ctx.library.is_done.read().unwrap() {
            status.push("updating library…".to_string());
        }
        if let Some(seconds) = crate::spotify_api::rate_limit_remaining() {
            status.push(format!("Spotify rate limit: try again in {seconds}s"));
        }
        if status.is_empty() {
            return;
        }
        let text = crate::tui::style::truncate(
            &status.join("  "),
            usize::from(area.width.saturating_sub(4)),
        );
        let x = area.right().saturating_sub(width(&text) as u16 + 2);
        put(
            frame.buffer_mut(),
            x,
            area.y,
            &text,
            usize::MAX,
            background.fg(ctx.palette.secondary),
        );
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        ctx: &Ctx,
        count: usize,
        show_cover: bool,
    ) {
        self.hits = Hits::default();
        if area.height >= HERO_MIN_ROWS {
            self.draw_hero(frame, area, ctx, count, show_cover);
        } else {
            self.draw_bar_layout(frame, area, ctx, count, show_cover);
        }
        for (area, control) in &self.hits.controls {
            pointer::clickable(*area);
            hints::add_below(
                *area,
                match control {
                    Control::Shuffle => Command::Shuffle(None),
                    Control::Previous => Command::Previous,
                    Control::PlayPause => Command::TogglePlay,
                    Control::Next => Command::Next,
                    Control::Repeat => Command::Repeat(None),
                },
            );
        }
        if let Some(like) = self.hits.like {
            hints::add_below(like, Command::Like);
        }
        if let Some(volume) = self.hits.volume {
            hints::add_row(volume, vec![Command::VolumeDown(5), Command::VolumeUp(5)]);
        }
        for hit in [self.hits.like, self.hits.progress].into_iter().flatten() {
            pointer::clickable(hit);
        }
        // The bar after the volume icon; the icon itself isn't clickable.
        if let Some(volume) = self.hits.volume {
            let bar = Rect::new(volume.x + 3, volume.y, volume.width.saturating_sub(3), 1);
            pointer::clickable(bar.intersection(volume));
        }
    }

    /// The hero beside its cover: for short heroes, where a cover above the controls would be
    /// tiny. The cover fills the height on the left; the track, progress, controls and volume
    /// stack up on the right.
    fn draw_hero_side(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        ctx: &Ctx,
        count: usize,
        url: &str,
    ) {
        let background = Style::new().bg(ctx.palette.background);
        let track = ctx.queue.get_current();
        let cover_rows = area.height.saturating_sub(2);
        let cover_width = images::square_width(cover_rows);
        images::cover(
            url,
            Rect::new(area.x + 2, area.y + 1, cover_width, cover_rows),
        );

        let left = area.x + 2 + cover_width + 4;
        let content_width = area.right().saturating_sub(left + 3).min(64);
        // Center the rows vertically next to the cover.
        let block = 11;
        let top = area.y + 1 + cover_rows.saturating_sub(block) / 2;
        self.draw_text(
            frame,
            Rect::new(left, top, content_width, 3),
            false,
            ctx,
            background,
        );

        let progress_row = top + 4;
        let bar = Rect::new(left, progress_row, content_width, 1);
        let (elapsed, total) = match &track {
            Some(t) => {
                let total = t.duration();
                (
                    (ctx.spotify.get_current_progress().as_millis() as u32).min(total),
                    total,
                )
            }
            None => (0, 0),
        };
        let fraction = if total > 0 {
            f64::from(elapsed) / f64::from(total)
        } else {
            0.0
        };
        Self::draw_bar(frame, bar, fraction, ctx, background);
        self.hits.progress = Some(bar);
        if track.is_some() {
            Self::draw_times(frame, bar, elapsed, total, ctx, background);
        }

        let controls_row = progress_row + 2;
        let bh = self.draw_hero_controls(frame, bar, controls_row, ctx);

        let volume_row = controls_row + bh + 1;
        if volume_row < area.bottom() && area.width >= VOLUME_BAR_WIDTH + 3 {
            let vx = left + content_width.saturating_sub(VOLUME_BAR_WIDTH + 3) / 2;
            put(
                frame.buffer_mut(),
                vx,
                volume_row,
                Self::volume_text(ctx),
                2,
                background.fg(ctx.palette.primary),
            );
            let vbar = Rect::new(vx + 3, volume_row, VOLUME_BAR_WIDTH, 1);
            Self::draw_volume(
                frame,
                vbar,
                f64::from(ctx.spotify.volume()) / f64::from(u16::MAX),
                ctx,
                background,
            );
            self.hits.volume = Some(Rect::new(vx, volume_row, VOLUME_BAR_WIDTH + 3, 1));
        }
        Self::draw_status(
            frame,
            Rect::new(area.x, area.y, area.width, 1),
            count,
            ctx,
            background,
        );
    }

    /// The elapsed and remaining times under the ends of the progress `bar`, if they fit.
    fn draw_times(
        frame: &mut Frame,
        bar: Rect,
        elapsed: u32,
        total: u32,
        ctx: &Ctx,
        background: Style,
    ) {
        let elapsed_text = duration(elapsed);
        let remaining = format!("-{}", duration(total - elapsed));
        if width(&elapsed_text) + width(&remaining) + 1 > usize::from(bar.width) {
            return;
        }
        let buf = frame.buffer_mut();
        let secondary = background.fg(ctx.palette.secondary);
        let row = bar.y + 1;
        put(buf, bar.x, row, &elapsed_text, 10, secondary);
        put(
            buf,
            bar.right() - width(&remaining) as u16,
            row,
            &remaining,
            10,
            secondary,
        );
    }

    /// The hero's controls in `row`, under the progress `bar`: shuffle at its left end, repeat
    /// at its right end, the rest centered between them, closer together when the bar is short.
    /// Returns their height.
    fn draw_hero_controls(&mut self, frame: &mut Frame, bar: Rect, row: u16, ctx: &Ctx) -> u16 {
        let (bw, bh) = Self::button_size();
        let ends = bar.width >= 5 * bw;
        if ends {
            self.draw_control(frame, Rect::new(bar.x, row, bw, bh), Control::Shuffle, ctx);
            self.draw_control(
                frame,
                Rect::new(bar.right() - bw, row, bw, bh),
                Control::Repeat,
                ctx,
            );
        }
        let gap = bw.min(bar.width.saturating_sub(5 * bw) / 4);
        let middle = 3 * bw + 2 * gap;
        let mx = bar.x + bar.width.saturating_sub(middle) / 2;
        for (i, control) in [Control::Previous, Control::PlayPause, Control::Next]
            .into_iter()
            .enumerate()
        {
            self.draw_control(
                frame,
                Rect::new(mx + i as u16 * (bw + gap), row, bw, bh),
                control,
                ctx,
            );
        }
        bh
    }

    fn draw_hero(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        ctx: &Ctx,
        count: usize,
        show_cover: bool,
    ) {
        // Short heroes put the cover beside the rest rather than above it.
        if show_cover
            && images::supported()
            && area.height < HERO_CHROME_ROWS + MIN_COVER_ROWS_ABOVE
            && area.height >= 12
            && area.width >= 70
            && let Some(url) = ctx
                .queue
                .get_current()
                .as_ref()
                .and_then(Playable::cover_url)
        {
            return self.draw_hero_side(frame, area, ctx, count, &url);
        }
        // Design V1: no fill of its own, the main background.
        let background = Style::new().bg(ctx.palette.background);
        let track = ctx.queue.get_current();

        // The cover as big as fits, square.
        let cover_url = track.as_ref().and_then(Playable::cover_url);
        let mut cover_rows = if show_cover && images::supported() && cover_url.is_some() {
            area.height.saturating_sub(HERO_CHROME_ROWS)
        } else {
            0
        };
        while cover_rows > 0 && images::square_width(cover_rows) + 4 > area.width {
            cover_rows -= 1;
        }
        if cover_rows < 4 {
            cover_rows = 0;
        }
        let block = HERO_CHROME_ROWS + cover_rows;
        let top = area.y + area.height.saturating_sub(block) / 2;
        if cover_rows > 0
            && let Some(url) = &cover_url
        {
            let w = images::square_width(cover_rows);
            images::cover(
                url,
                Rect::new(area.x + (area.width - w) / 2, top + 1, w, cover_rows),
            );
        }

        let text_top = top + 1 + cover_rows + 1;
        let content_width = area.width.saturating_sub(6).min(64);
        let left = area.x + (area.width - content_width) / 2;
        self.draw_text(
            frame,
            Rect::new(left, text_top, content_width, 3),
            true,
            ctx,
            background,
        );

        // Progress, with the times under its ends.
        let progress_row = text_top + 4;
        let bar = Rect::new(left, progress_row, content_width, 1);
        let (elapsed, total) = match &track {
            Some(t) => {
                let total = t.duration();
                (
                    (ctx.spotify.get_current_progress().as_millis() as u32).min(total),
                    total,
                )
            }
            None => (0, 0),
        };
        let fraction = if total > 0 {
            f64::from(elapsed) / f64::from(total)
        } else {
            0.0
        };
        Self::draw_bar(frame, bar, fraction, ctx, background);
        self.hits.progress = Some(bar);
        if track.is_some() {
            Self::draw_times(frame, bar, elapsed, total, ctx, background);
        }

        let controls_row = progress_row + 2;
        let bh = self.draw_hero_controls(frame, bar, controls_row, ctx);

        // Volume, centered under the controls.
        let volume_row = controls_row + bh + 1;
        if volume_row < area.bottom() && area.width >= VOLUME_BAR_WIDTH + 3 {
            let vx = area.x + area.width.saturating_sub(VOLUME_BAR_WIDTH + 3) / 2;
            put(
                frame.buffer_mut(),
                vx,
                volume_row,
                Self::volume_text(ctx),
                2,
                background.fg(ctx.palette.primary),
            );
            let vbar = Rect::new(vx + 3, volume_row, VOLUME_BAR_WIDTH, 1);
            Self::draw_volume(
                frame,
                vbar,
                f64::from(ctx.spotify.volume()) / f64::from(u16::MAX),
                ctx,
                background,
            );
            self.hits.volume = Some(Rect::new(vx, volume_row, VOLUME_BAR_WIDTH + 3, 1));
        }

        Self::draw_status(
            frame,
            Rect::new(area.x, area.y, area.width, 1),
            count,
            ctx,
            background,
        );
    }

    fn draw_bar_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        ctx: &Ctx,
        count: usize,
        show_cover: bool,
    ) {
        let styles = Styles(&ctx.palette);
        let background = styles.panel();
        fill(frame.buffer_mut(), area, background);
        if area.height == 0 {
            return;
        }
        let track = ctx.queue.get_current();
        let mid = area.y + area.height / 2;

        let mut text_x = area.x + 2;
        if show_cover
            && images::supported()
            && area.height >= 3
            && let Some(url) = track.as_ref().and_then(Playable::cover_url)
        {
            let w = images::square_width(area.height);
            images::cover(&url, Rect::new(area.x + 1, area.y, w, area.height));
            text_x = area.x + w + 3;
        }

        let center_width = (area.width * 2 / 5).clamp(30, 80).min(area.width);
        let center_x = area.x + area.width.saturating_sub(center_width) / 2;
        let text_width = center_x.saturating_sub(text_x + 2);
        // Too narrow for the track beside the controls: the title goes on a row of its own
        // above them, when there are rows for it.
        let stacked = text_width < MIN_BAR_TEXT_WIDTH;
        let (controls_row, progress_row) = if stacked && area.height >= 3 {
            // The artists too, when there's a row for them.
            let rows = if area.height >= 4 { 2 } else { 1 };
            let text_area = Rect::new(
                text_x,
                mid - rows,
                area.right().saturating_sub(text_x + 2),
                rows,
            );
            self.draw_text(frame, text_area, false, ctx, background);
            (mid, mid + 1)
        } else {
            if !stacked {
                let top = mid.saturating_sub(1).max(area.y);
                let text_area = Rect::new(text_x, top, text_width, (area.bottom() - top).min(3));
                self.draw_text(frame, text_area, false, ctx, background);
            }
            (
                mid.saturating_sub(1).max(area.y),
                (mid + 1).min(area.bottom() - 1),
            )
        };

        // Controls over the progress bar, K3 order: shuffle, previous, play, next, repeat.
        let (bw, bh) = if image_controls() && area.height >= 4 && !stacked {
            Self::button_size()
        } else {
            (3, 1)
        };
        let gap = if 5 * bw + 4 * 2 <= area.width { 2 } else { 0 };
        let controls_width = 5 * bw + 4 * gap;
        let cx = area.x + area.width.saturating_sub(controls_width) / 2;
        let row = if bh > 1 {
            mid.saturating_sub(2).max(area.y)
        } else {
            controls_row
        };
        for (i, control) in [
            Control::Shuffle,
            Control::Previous,
            Control::PlayPause,
            Control::Next,
            Control::Repeat,
        ]
        .into_iter()
        .enumerate()
        {
            let x = cx + i as u16 * (bw + gap);
            if x + bw > area.right() {
                break;
            }
            self.draw_control(frame, Rect::new(x, row, bw, bh), control, ctx);
        }

        // A single row only has room for the controls.
        if progress_row == row {
            return;
        }
        let bar = Rect::new(
            center_x + 6,
            progress_row,
            center_width.saturating_sub(13),
            1,
        );
        if let Some(t) = &track {
            let total = t.duration();
            let elapsed = (ctx.spotify.get_current_progress().as_millis() as u32).min(total);
            let buf = frame.buffer_mut();
            let secondary = background.fg(ctx.palette.secondary);
            let e = duration(elapsed);
            put(
                buf,
                bar.x.saturating_sub(width(&e) as u16 + 1),
                progress_row,
                &e,
                10,
                secondary,
            );
            put(
                buf,
                bar.right() + 1,
                progress_row,
                &format!("-{}", duration(total - elapsed)),
                10,
                secondary,
            );
            let fraction = if total > 0 {
                f64::from(elapsed) / f64::from(total)
            } else {
                0.0
            };
            Self::draw_bar(frame, bar, fraction, ctx, background);
        } else {
            Self::draw_bar(frame, bar, 0.0, ctx, background);
        }
        self.hits.progress = Some(bar);

        let right_space = area.right().saturating_sub(center_x + center_width);
        if right_space >= VOLUME_BAR_WIDTH + 6 {
            let vx = area.right().saturating_sub(VOLUME_BAR_WIDTH + 5);
            put(
                frame.buffer_mut(),
                vx,
                mid,
                Self::volume_text(ctx),
                2,
                background.fg(ctx.palette.primary),
            );
            Self::draw_volume(
                frame,
                Rect::new(vx + 3, mid, VOLUME_BAR_WIDTH, 1),
                f64::from(ctx.spotify.volume()) / f64::from(u16::MAX),
                ctx,
                background,
            );
            self.hits.volume = Some(Rect::new(vx, mid, VOLUME_BAR_WIDTH + 3, 1));
        }
        Self::draw_status(
            frame,
            Rect::new(area.x, mid.saturating_sub(1).max(area.y), area.width, 1),
            count,
            ctx,
            background,
        );
    }

    fn activate(control: Control, ctx: &Ctx) {
        match control {
            Control::Shuffle => ctx.queue.set_shuffle(!ctx.queue.get_shuffle()),
            Control::Previous => {
                if ctx.spotify.get_current_progress() < Duration::from_secs(5) {
                    ctx.queue.previous();
                } else {
                    ctx.spotify.seek(0);
                }
            }
            Control::PlayPause => ctx.queue.toggleplayback(),
            Control::Next => ctx.queue.next(true),
            Control::Repeat => ctx.queue.set_repeat(match ctx.queue.get_repeat() {
                RepeatSetting::None => RepeatSetting::RepeatPlaylist,
                RepeatSetting::RepeatPlaylist => RepeatSetting::RepeatTrack,
                RepeatSetting::RepeatTrack => RepeatSetting::None,
            }),
        }
    }

    pub fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Outcome {
        let at = Position::new(event.column, event.row);
        let in_rect = |r: &Option<Rect>| r.is_some_and(|r| r.contains(at));
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, control)) = self.hits.controls.iter().find(|(r, _)| r.contains(at))
                {
                    Self::activate(*control, ctx);
                } else if in_rect(&self.hits.like) {
                    if let Some(Playable::Track(track)) = ctx.queue.get_current() {
                        ctx.library.toggle_saved_track_in_background(&track);
                    }
                } else if let Some(bar) = self.hits.progress.filter(|r| r.contains(at)) {
                    if let Some(track) = ctx.queue.get_current() {
                        let fraction = f64::from(at.x - bar.x) / f64::from(bar.width.max(1));
                        ctx.spotify
                            .seek((f64::from(track.duration()) * fraction) as u32);
                    }
                } else if let Some(volume) = self.hits.volume.filter(|r| r.contains(at)) {
                    let bar_x = volume.x + 3;
                    if at.x >= bar_x {
                        Self::set_volume_at(at.x, bar_x, ctx);
                        self.dragging_volume = true;
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging_volume => {
                if let Some(volume) = self.hits.volume {
                    Self::set_volume_at(at.x, volume.x + 3, ctx);
                }
            }
            MouseEventKind::Up(_) if self.dragging_volume => self.dragging_volume = false,
            MouseEventKind::ScrollUp if in_rect(&self.hits.volume) => {
                ctx.spotify
                    .set_volume(ctx.spotify.volume().saturating_add(VOLUME_PERCENT), true);
            }
            MouseEventKind::ScrollDown if in_rect(&self.hits.volume) => {
                ctx.spotify
                    .set_volume(ctx.spotify.volume().saturating_sub(VOLUME_PERCENT), true);
            }
            MouseEventKind::ScrollUp if in_rect(&self.hits.progress) => {
                ctx.spotify.seek_relative(-5000)
            }
            MouseEventKind::ScrollDown if in_rect(&self.hits.progress) => {
                ctx.spotify.seek_relative(5000)
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Consumed
    }
}
