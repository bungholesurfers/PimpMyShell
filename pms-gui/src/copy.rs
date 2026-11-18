//! What the settings say: each setting's title and description, the same in both windows (egui in main.rs, WinUI 3 in
//! winui.rs). Written as Microsoft's style guide has it: sentence case, "you", short and active, no period after a
//! fragment.
//!
//! A setting is known by its id, `table.key` as the config file has it (`meter.unit`, `hotmic.color`), or `key` for
//! a top-level one; a few say different things for different meter styles (`meter.names.disks`).

use pms::lang::settings::{self as s, Id};

/// The title and description of setting `id`. An id with nothing written here shows itself.
pub fn setting(id: &str) -> (&str, &str) {
    // The light look's are the dark look's, in a table of their own.
    if let Some(rest) = id.strip_prefix("look.light.") {
        return setting(match rest {
            "tint" => "look.tint",
            "grain" => "look.grain",
            "gaps" => "look.gaps",
            _ => id,
        });
    }
    match id {
        // General
        "language" => (s::LANGUAGE.t(), s::LANGUAGE_HELP.t()),
        "theme" => (s::THEME.t(), s::THEME_HELP.t()),
        "meter_height" => (s::METER_HEIGHT.t(), s::METER_HEIGHT_HELP.t()),
        "bar_height" => (s::BAR_HEIGHT.t(), s::BAR_HEIGHT_HELP.t()),
        "interval_ms" => (s::INTERVAL_MS.t(), s::INTERVAL_MS_HELP.t()),
        "software_gpus" => (s::SOFTWARE_GPUS.t(), s::SOFTWARE_GPUS_HELP.t()),
        "rest_when_covered" => (s::REST_WHEN_COVERED.t(), s::REST_WHEN_COVERED_HELP.t()),
        "rest_for_games" => (s::REST_FOR_GAMES.t(), s::REST_FOR_GAMES_HELP.t()),
        "max_fps" => (s::MAX_FPS.t(), s::MAX_FPS_HELP.t()),
        "battery_fps" => (s::BATTERY_FPS.t(), s::BATTERY_FPS_HELP.t()),
        "value_font" => (s::VALUE_FONT.t(), s::VALUE_FONT_HELP.t()),
        "update_check" => (s::UPDATE_CHECK.t(), s::UPDATE_CHECK_HELP.t()),
        "update_channel" => (s::UPDATE_CHANNEL.t(), s::UPDATE_CHANNEL_HELP.t()),
        "update_warn" => (s::UPDATE_WARN.t(), s::UPDATE_WARN_HELP.t()),

        "notify_explorer" => (s::NOTIFY_EXPLORER.t(), s::NOTIFY_EXPLORER_HELP.t()),

        // TV and Twatch TV
        "shuffle_seconds" => (s::SHUFFLE_SECONDS.t(), s::SHUFFLE_SECONDS_HELP.t()),
        "shuffle_jitter" => (s::SHUFFLE_JITTER.t(), s::SHUFFLE_JITTER_HELP.t()),
        "console_video" => (s::CONSOLE_VIDEO.t(), s::CONSOLE_VIDEO_HELP.t()),
        "console_autostart" => (s::CONSOLE_AUTOSTART.t(), s::CONSOLE_AUTOSTART_HELP.t()),
        "console_demos" => (s::CONSOLE_DEMOS.t(), s::CONSOLE_DEMOS_HELP.t()),
        "demos.when" => (s::DEMOS_WHEN.t(), s::DEMOS_WHEN_HELP.t()),
        "demos.every_hours" => (s::DEMOS_EVERY_HOURS.t(), s::DEMOS_EVERY_HOURS_HELP.t()),
        "demos.at_once" => (s::DEMOS_AT_ONCE.t(), s::DEMOS_AT_ONCE_HELP.t()),
        "demos.gap_seconds" => (s::DEMOS_GAP_SECONDS.t(), s::DEMOS_GAP_SECONDS_HELP.t()),
        "demos.most" => (s::DEMOS_MOST.t(), s::DEMOS_MOST_HELP.t()),
        "demos.list_days" => (s::DEMOS_LIST_DAYS.t(), s::DEMOS_LIST_DAYS_HELP.t()),
        "demos.fired_at_least" => (s::DEMOS_FIRED_AT_LEAST.t(), s::DEMOS_FIRED_AT_LEAST_HELP.t()),
        "tv_intro" => (s::TV_INTRO.t(), s::TV_INTRO_HELP.t()),
        "tv_intro_from" => (s::TV_INTRO_FROM.t(), s::TV_INTRO_FROM_HELP.t()),
        "tv_intro_to" => (s::TV_INTRO_TO.t(), s::TV_INTRO_TO_HELP.t()),
        "iss.online" => (s::ISS_ONLINE.t(), s::ISS_ONLINE_HELP.t()),
        "iss.notify_flush" => (s::ISS_NOTIFY_FLUSH.t(), s::ISS_NOTIFY_FLUSH_HELP.t()),
        "iss.bumps" => (s::ISS_BUMPS.t(), s::ISS_BUMPS_HELP.t()),
        "twatch.media_controls" => (s::TWATCH_MEDIA_CONTROLS.t(), s::TWATCH_MEDIA_CONTROLS_HELP.t()),
        "twatch.pictures" => (s::TWATCH_PICTURES.t(), s::TWATCH_PICTURES_HELP.t()),
        "twatch.volume" => (s::TWATCH_VOLUME.t(), s::TWATCH_VOLUME_HELP.t()),
        "twatch.metered_pictures" => (s::TWATCH_METERED_PICTURES.t(), s::TWATCH_METERED_PICTURES_HELP.t()),
        "twatch.notify_live" => (s::TWATCH_NOTIFY_LIVE.t(), s::TWATCH_NOTIFY_LIVE_HELP.t()),
        "twatch.top_quality" => (s::TWATCH_TOP_QUALITY.t(), s::TWATCH_TOP_QUALITY_HELP.t()),
        "twatch.frame_rate" => (s::TWATCH_FRAME_RATE.t(), s::TWATCH_FRAME_RATE_HELP.t()),
        "twatch.more_sets" => (s::TWATCH_MORE_SETS.t(), s::TWATCH_MORE_SETS_HELP.t()),
        "twatch.more_sets_on" => (s::TWATCH_MORE_SETS_ON.t(), s::TWATCH_MORE_SETS_ON_HELP.t()),
        "twatch.higher_favorite" => (s::TWATCH_HIGHER_FAVORITE.t(), s::TWATCH_HIGHER_FAVORITE_HELP.t()),
        "twatch.check_seconds" => (s::TWATCH_CHECK_SECONDS.t(), s::TWATCH_CHECK_SECONDS_HELP.t()),
        "twatch.check_extra_seconds" => (s::TWATCH_CHECK_EXTRA_SECONDS.t(), s::TWATCH_CHECK_EXTRA_SECONDS_HELP.t()),

        // Meters
        "meter.label" => (s::METER_LABEL.t(), s::METER_LABEL_HELP.t()),
        "meter.group" => (s::METER_GROUP.t(), s::METER_GROUP_HELP.t()),
        "meter.sideways" => (s::METER_SIDEWAYS.t(), s::METER_SIDEWAYS_HELP.t()),
        "meter.snug" => (s::METER_SNUG.t(), s::METER_SNUG_HELP.t()),
        "meter.caption" => (s::METER_CAPTION.t(), s::METER_CAPTION_HELP.t()),
        "meter.style" => (s::METER_STYLE.t(), s::METER_STYLE_HELP.t()),
        "meter.expr" => (s::METER_EXPR.t(), s::METER_EXPR_HELP.t()),
        "meter.names" => (s::METER_NAMES.t(), s::METER_NAMES_HELP.t()),
        "meter.names.disks" => (s::METER_NAMES_DISKS.t(), s::METER_NAMES_DISKS_HELP.t()),
        "meter.names.nics" => (s::METER_NAMES_NICS.t(), s::METER_NAMES_NICS_HELP.t()),
        "meter.colors" => (s::METER_COLORS.t(), s::METER_COLORS_HELP.t()),
        "meter.colors.cores" => (s::METER_COLORS_CORES.t(), s::METER_COLORS_CORES_HELP.t()),
        "meter.colors.disks" => (s::METER_COLORS_DISKS.t(), s::METER_COLORS_DISKS_HELP.t()),
        "meter.colors.nics" => (s::METER_COLORS_NICS.t(), s::METER_COLORS_NICS_HELP.t()),
        "meter.color" => (s::METER_COLOR.t(), s::METER_COLOR_HELP.t()),
        "meter.max" => (s::METER_MAX.t(), s::METER_MAX_HELP.t()),
        "meter.unit" => (s::METER_UNIT.t(), s::METER_UNIT_HELP.t()),
        "meter.decimals" => (s::METER_DECIMALS.t(), s::METER_DECIMALS_HELP.t()),
        "meter.interval_ms" => (s::METER_INTERVAL_MS.t(), s::METER_INTERVAL_MS_HELP.t()),
        "meter.value" => (s::METER_VALUE.t(), s::METER_VALUE_HELP.t()),
        "meter.tooltip" => (s::METER_TOOLTIP.t(), s::METER_TOOLTIP_HELP.t()),
        "meter.glow" => (s::METER_GLOW.t(), s::METER_GLOW_HELP.t()),
        "meter.kernel" => (s::METER_KERNEL.t(), s::METER_KERNEL_HELP.t()),
        "meter.bar_width" => (s::METER_BAR_WIDTH.t(), s::METER_BAR_WIDTH_HELP.t()),
        "meter.vendors" => (s::METER_VENDORS.t(), s::METER_VENDORS_HELP.t()),
        "meter.vendor_label" => (s::METER_VENDOR_LABEL.t(), s::METER_VENDOR_LABEL_HELP.t()),
        "meter.hold_seconds" => (s::METER_HOLD_SECONDS.t(), s::METER_HOLD_SECONDS_HELP.t()),
        "meter.hold_seconds.disks" => (s::METER_HOLD_SECONDS_DISKS.t(), s::METER_HOLD_SECONDS_DISKS_HELP.t()),
        "meter.hold_seconds.nics" => (s::METER_HOLD_SECONDS_NICS.t(), s::METER_HOLD_SECONDS_NICS_HELP.t()),
        "meter.engines_fall_off" => (s::METER_ENGINES_FALL_OFF.t(), s::METER_ENGINES_FALL_OFF_HELP.t()),
        "meter.per_gpu" => (s::METER_PER_GPU.t(), s::METER_PER_GPU_HELP.t()),
        "meter.label_inside" => (s::METER_LABEL_INSIDE.t(), s::METER_LABEL_INSIDE_HELP.t()),
        "meter.slice_glow" => (s::METER_SLICE_GLOW.t(), s::METER_SLICE_GLOW_HELP.t()),
        "meter.slice_glow_extend" => (s::METER_SLICE_GLOW_EXTEND.t(), s::METER_SLICE_GLOW_EXTEND_HELP.t()),
        "meter.slice_glow_radius" => (s::METER_SLICE_GLOW_RADIUS.t(), s::METER_SLICE_GLOW_RADIUS_HELP.t()),
        "meter.slice_glow_strength" => (s::METER_SLICE_GLOW_STRENGTH.t(), s::METER_SLICE_GLOW_STRENGTH_HELP.t()),
        "meter.slice_glow_color" => (s::METER_SLICE_GLOW_COLOR.t(), s::METER_SLICE_GLOW_COLOR_HELP.t()),
        "meter.slice_glow_pulse_ms" => (s::METER_SLICE_GLOW_PULSE_MS.t(), s::METER_SLICE_GLOW_PULSE_MS_HELP.t()),
        "meter.pressure" => (s::METER_PRESSURE.t(), s::METER_PRESSURE_HELP.t()),
        "meter.time_axis" => (s::METER_TIME_AXIS.t(), s::METER_TIME_AXIS_HELP.t()),
        "meter.history_seconds" => (s::METER_HISTORY_SECONDS.t(), s::METER_HISTORY_SECONDS_HELP.t()),
        "meter.show_above" => (s::METER_SHOW_ABOVE.t(), s::METER_SHOW_ABOVE_HELP.t()),
        "meter.show_when" => (s::METER_SHOW_WHEN.t(), s::METER_SHOW_WHEN_HELP.t()),

        // Counters
        "counter.name" => (s::COUNTER_NAME.t(), s::COUNTER_NAME_HELP.t()),
        "counter.path" => (s::COUNTER_PATH.t(), s::COUNTER_PATH_HELP.t()),
        "counter.agg" => (s::COUNTER_AGG.t(), s::COUNTER_AGG_HELP.t()),
        "counter.default" => (s::COUNTER_DEFAULT.t(), s::COUNTER_DEFAULT_HELP.t()),

        // Taskbar look
        "look.tint" => (s::LOOK_TINT.t(), s::LOOK_TINT_HELP.t()),
        "look.grain" => (s::LOOK_GRAIN.t(), s::LOOK_GRAIN_HELP.t()),
        "look.gaps" => (s::LOOK_GAPS.t(), s::LOOK_GAPS_HELP.t()),
        "start_glow.color" => (s::START_GLOW_COLOR.t(), s::START_GLOW_COLOR_HELP.t()),
        "start_glow.hover" => (s::START_GLOW_HOVER.t(), s::START_GLOW_HOVER_HELP.t()),
        "start_glow.press" => (s::START_GLOW_PRESS.t(), s::START_GLOW_PRESS_HELP.t()),
        "start_glow.open" => (s::START_GLOW_OPEN.t(), s::START_GLOW_OPEN_HELP.t()),
        "start_glow.hover_ms" => (s::START_GLOW_HOVER_MS.t(), s::START_GLOW_HOVER_MS_HELP.t()),
        "start_glow.press_ms" => (s::START_GLOW_PRESS_MS.t(), s::START_GLOW_PRESS_MS_HELP.t()),
        "start_glow.hover_radius" => (s::START_GLOW_HOVER_RADIUS.t(), s::START_GLOW_HOVER_RADIUS_HELP.t()),
        "start_glow.press_radius" => (s::START_GLOW_PRESS_RADIUS.t(), s::START_GLOW_PRESS_RADIUS_HELP.t()),
        "app_glow.hover" => (s::APP_GLOW_HOVER.t(), s::APP_GLOW_HOVER_HELP.t()),
        "app_glow.press" => (s::APP_GLOW_PRESS.t(), s::APP_GLOW_PRESS_HELP.t()),
        "app_glow.hover_ms" => (s::APP_GLOW_HOVER_MS.t(), s::APP_GLOW_HOVER_MS_HELP.t()),
        "app_glow.press_ms" => (s::APP_GLOW_PRESS_MS.t(), s::APP_GLOW_PRESS_MS_HELP.t()),
        "app_glow.hover_radius" => (s::APP_GLOW_HOVER_RADIUS.t(), s::APP_GLOW_HOVER_RADIUS_HELP.t()),
        "app_glow.press_radius" => (s::APP_GLOW_PRESS_RADIUS.t(), s::APP_GLOW_PRESS_RADIUS_HELP.t()),

        // Hot Mic
        "hotmic.color" => (s::HOTMIC_COLOR.t(), s::HOTMIC_COLOR_HELP.t()),
        "hotmic.period_ms" => (s::HOTMIC_PERIOD_MS.t(), s::HOTMIC_PERIOD_MS_HELP.t()),
        "hotmic.low" => (s::HOTMIC_LOW.t(), s::HOTMIC_LOW_HELP.t()),
        "hotmic.high" => (s::HOTMIC_HIGH.t(), s::HOTMIC_HIGH_HELP.t()),
        "hotmic.broad_ms" => (s::HOTMIC_BROAD_MS.t(), s::HOTMIC_BROAD_MS_HELP.t()),
        "hotmic.ignore" => (s::HOTMIC_IGNORE.t(), s::HOTMIC_IGNORE_HELP.t()),
        "hotmic.position" => (s::HOTMIC_POSITION.t(), s::HOTMIC_POSITION_HELP.t()),

        // Sandbox
        // Each by its program's file name, as Windows shows the process: not something that's translated.
        "sandbox.fetch" => ("pms-fetch.exe", s::SANDBOX_FETCH_HELP.t()),
        "sandbox.twatch" => ("pms-twatch.exe", s::SANDBOX_TWATCH_HELP.t()),
        "sandbox.streamlink" => ("python.exe", s::SANDBOX_STREAMLINK_HELP.t()),
        "sandbox.poom" => ("pms-poom.exe", s::SANDBOX_POOM_HELP.t()),
        "efficiency.daemon" => (s::EFFICIENCY_DAEMON.t(), s::EFFICIENCY_DAEMON_HELP.t()),
        "efficiency.twatch" => (s::EFFICIENCY_TWATCH.t(), s::EFFICIENCY_TWATCH_HELP.t()),
        "efficiency.helpers" => (s::EFFICIENCY_HELPERS.t(), s::EFFICIENCY_HELPERS_HELP.t()),
        "efficiency.pimpbox" => (s::EFFICIENCY_PIMPBOX.t(), s::EFFICIENCY_PIMPBOX_HELP.t()),

        // Notifications
        "toasts.duration_ms" => (s::TOASTS_DURATION_MS.t(), s::TOASTS_DURATION_MS_HELP.t()),
        "toasts.confirm_clicks" => (s::TOASTS_CONFIRM_CLICKS.t(), s::TOASTS_CONFIRM_CLICKS_HELP.t()),
        "toasts.position" => (s::TOASTS_POSITION.t(), s::TOASTS_POSITION_HELP.t()),
        "toasts.style" => (s::TOASTS_STYLE.t(), s::TOASTS_STYLE_HELP.t()),
        "toasts.ignore" => (s::TOASTS_IGNORE.t(), s::TOASTS_IGNORE_HELP.t()),

        _ => (id, ""),
    }
}

/// Where setting `id` is in pms.toml, written as the file has it, so that it can be found there: `language` at the top,
/// `[twatch] volume` in a table, `[[meter]] names` in each meter (`meter.names.disks` is a kind of `names`), `[backdrop] tint`
/// or `[backdrop.light] tint` for the looks, `[counters] path` inside a counter's own `{ ... }`.
pub fn key(id: &str) -> String {
    let mut parts = id.split('.');
    let first = parts.next().unwrap_or(id);
    let Some(second) = parts.next() else { return first.to_string() };
    match first {
        "meter" => format!("[[meter]] {second}"),
        "counter" if second == "name" => "[counters]".to_string(),
        "counter" => format!("[counters] {second}"),
        "look" if second == "light" => format!("[backdrop.light] {}", parts.next().unwrap_or_default()),
        "look" => format!("[backdrop] {second}"),
        table => format!("[{table}] {second}"),
    }
}

/// Why the classic window is showing, where nobody asked for it.
pub const CLASSIC_NO_RUNTIME: Id = s::CLASSIC_NO_RUNTIME;
pub const CLASSIC_FAILED: Id = s::CLASSIC_FAILED;
pub const RUNTIME_DOWNLOAD: (Id, &str) = (s::RUNTIME_DOWNLOAD, "https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads");

/// Lines of the settings file that aren't settings, which are left out so that the rest loads.
pub const IGNORED_LINES: Id = s::IGNORED_LINES;

/// Why the favorites and the network settings have files of their own.
pub const PRIVATE_FILES: Id = s::PRIVATE_FILES;

/// What's in the logs, and what isn't (logfile.rs, the daemon's lines on toasts, system.rs's report): its title, what's left out,
/// and what each file can say about you. Kept to what the code logs: a new kind of line that says something about somebody goes here.
pub const LOG_PRIVACY: (Id, Id) = (s::LOG_PRIVACY, s::LOG_PRIVACY_ABOUT);
pub const LOG_CONTENTS: &[(Id, &[Id])] = &[
    (s::LOG_DAEMON, &[s::LOG_DAEMON_1, s::LOG_DAEMON_2, s::LOG_DAEMON_3, s::LOG_DAEMON_4, s::LOG_DAEMON_5, s::LOG_DAEMON_6, s::LOG_DAEMON_7, s::LOG_DAEMON_8, s::LOG_DAEMON_9]),
    (s::LOG_TAP, &[s::LOG_TAP_1, s::LOG_TAP_2]),
    (s::LOG_REPORT, &[s::LOG_REPORT_1, s::LOG_REPORT_2]),
];

/// LOG_CONTENTS, as bulleted lines.
pub fn log_contents() -> String {
    LOG_CONTENTS.iter().map(|(file, lines)| format!("{}\n{}", file.t(), lines.iter().map(|l| format!("  •  {}", l.t())).collect::<Vec<_>>().join("\n"))).collect::<Vec<_>>().join("\n\n")
}

/// The small print for what floats behind the settings window (dollars.rs).
pub const LEGAL: &str = include_str!("legal.txt");

/// The speed demos known for the game in `iwad`, in map order, then by category.
pub fn demos_known(iwad: &std::path::Path) -> Vec<pms::demos::Demo> {
    let mut known = pms::demos::known(iwad);
    known.sort_by(|a, b| (&a.level, &a.category).cmp(&(&b.level, &b.category)));
    known
}

/// How far finding and checking demos has got, and how many are shown, of those that play, with `fired_at_least`.
pub fn demos_so_far(known: &[pms::demos::Demo], fired_at_least: u32) -> String {
    let checked = known.iter().filter(|d| d.checked).count();
    let plays = known.iter().filter(|d| d.plays).count();
    let shown = known.iter().filter(|d| pms::demos::shown(d, fired_at_least)).count();
    let waiting = known.len() - checked;
    if known.is_empty() {
        s::PIMPBOX_DEMOS_LOOKING.t().to_string()
    } else {
        s::PIMPBOX_DEMOS_SO_FAR.with(&[("plays", &plays), ("checked", &checked), ("waiting", &waiting), ("shown", &shown)])
    }
}

/// A demo, as its map, category and time: E1M2 · UV Speed · 0:21.94.
pub fn demo_title(d: &pms::demos::Demo) -> String {
    format!("{} · {} · {}", d.level, d.category, d.time)
}

/// Who played it, and with what, and how much shooting there is in it.
pub fn demo_about(d: &pms::demos::Demo) -> String {
    s::PIMPBOX_DEMO_BY.with(&[("players", &d.players.join(", ")), ("engine", &d.engine), ("fired", &d.fired)])
}

/// Every program that does each use of the network, by its file name, and what it does of it: for the Network page, so that
/// what goes online is never only "PimpMyShell". A new use, or a new program doing one, goes here as well as in net::USES.
pub fn programs(what: pms::net::Use) -> &'static [(&'static str, Id)] {
    use pms::net::Use;
    match what {
        Use::Live => &[("pms-fetch.exe", s::NET_LIVE_FETCH)],
        Use::Streams | Use::Video => &[("python.exe", s::NET_STREAMS_STREAMLINK), ("pms-fetch.exe", s::NET_STREAMS_FETCH), ("pms-twatch.exe", s::NET_STREAMS_TWATCH), ("pms-fetch.exe", s::NET_RELAY)],
        Use::Updates => &[("pms-fetch.exe", s::NET_UPDATES_FETCH)],
        Use::Downloads => &[("pms-fetch.exe", s::NET_DOWNLOADS_FETCH)],
        Use::Iss => &[("pms-fetch.exe", s::NET_ISS_FETCH)],
    }
}

/// Choices: each value as the file has it, and what the window calls it.
pub const THEMES: &[(&str, Id)] = &[("auto", s::THEMES_AUTO), ("dark", s::THEMES_DARK), ("light", s::THEMES_LIGHT)];
pub const HEIGHTS: &[(&str, Id)] = &[("compact", s::HEIGHTS_COMPACT), ("tall", s::HEIGHTS_TALL), ("full", s::HEIGHTS_FULL)];
pub const CHANNELS: &[(&str, Id)] = &[("stable", s::CHANNELS_STABLE), ("latest", s::CHANNELS_LATEST)];
pub const CLEAR_SHADERS: (Id, Id) = (s::CLEAR_SHADERS, s::CLEAR_SHADERS_ABOUT);
pub const CLEAR_SHADERS_ASK: Id = s::CLEAR_SHADERS_ASK;
pub const DEMOS_WHEN: &[(&str, Id)] = &[("always", s::DEMOS_WHEN_ALWAYS), ("console", s::DEMOS_WHEN_CONSOLE)];
pub const CONSOLE_VIDEO: &[(&str, Id)] = &[("480p", s::CONSOLE_VIDEO_480P), ("720p", s::CONSOLE_VIDEO_720P), ("1080i", s::CONSOLE_VIDEO_1080I)];
pub const TOAST_STYLES: &[(&str, Id)] = &[("standard", s::TOAST_STYLES_STANDARD), ("pimped", s::TOAST_STYLES_PIMPED)];
pub const TOAST_POSITIONS: &[(&str, Id)] = &[("clock", s::TOAST_POSITIONS_CLOCK), ("tray", s::TOAST_POSITIONS_TRAY), ("tv", s::TOAST_POSITIONS_TV), ("mic", s::TOAST_POSITIONS_MIC)];
pub const CONFIRMS: &[(&str, Id)] = &[("card", s::CONFIRMS_CARD), ("window", s::CONFIRMS_WINDOW), ("off", s::CONFIRMS_OFF)];
pub const MIC_POSITIONS: &[(&str, Id)] = &[("tray", s::MIC_POSITIONS_TRAY), ("start", s::MIC_POSITIONS_START)];
pub const STYLES: &[(&str, Id)] = &[
    ("history", s::STYLES_HISTORY),
    ("bar", s::STYLES_BAR),
    ("pie", s::STYLES_PIE),
    ("bars", s::STYLES_BARS),
    ("cores", s::STYLES_CORES),
    ("engines", s::STYLES_ENGINES),
    ("disks", s::STYLES_DISKS),
    ("nics", s::STYLES_NICS),
];
pub const UNITS: &[(&str, Id)] = &[("", s::UNITS_NONE), ("%", s::UNITS_PERCENT), ("bytes", s::UNITS_BYTES), ("bytes/s", s::UNITS_BYTES_S), ("bits/s", s::UNITS_BITS_S)];
pub const GLOWS: &[(&str, Id)] = &[("level", s::GLOWS_LEVEL), ("heat", s::GLOWS_HEAT), ("off", s::GLOWS_OFF)];
pub const KERNELS: &[(&str, Id)] = &[("bottom", s::KERNELS_BOTTOM), ("top", s::KERNELS_TOP)];
pub const VENDOR_LABELS: &[(&str, Id)] = &[("short", s::VENDOR_LABELS_SHORT), ("ticker", s::VENDOR_LABELS_TICKER)];
pub const INSIDE: &[(&str, Id)] = &[("never", s::INSIDE_NEVER), ("auto", s::INSIDE_AUTO), ("always", s::INSIDE_ALWAYS)];
pub const TIME_AXES: &[(&str, Id)] = &[("log", s::TIME_AXES_LOG), ("linear", s::TIME_AXES_LINEAR)];
pub const AGGS: &[(&str, Id)] = &[("sum", s::AGGS_SUM), ("max", s::AGGS_MAX), ("min", s::AGGS_MIN), ("avg", s::AGGS_AVG), ("engine", s::AGGS_ENGINE)];

/// Pages: title, and what's on them.
pub const GENERAL: (Id, Id) = (s::GENERAL, s::GENERAL_ABOUT);
pub const METERS: (Id, Id) = (s::METERS, s::METERS_ABOUT);
/// When PimpMyShell gets out of the way, and what it does by itself.
pub const POWER: (Id, Id) = (s::POWER, s::POWER_ABOUT);
pub const POWER_BY_ITSELF: Id = s::POWER_BY_ITSELF;
/// Under Twatch TV's settings: where its pictures went.
pub const TWATCH_POWER: Id = s::TWATCH_POWER;
/// The logs' page.
pub const LOGS: (Id, Id) = (s::LOGS, s::LOGS_ABOUT);
/// The page of what's along the taskbar, and its notes.
pub const WIDGETS: (Id, Id) = (s::WIDGETS, s::WIDGETS_ABOUT);
pub const WIDGET_MIC_FIRST: Id = s::WIDGET_MIC_FIRST;
pub const WIDGET_MIC_LAST: Id = s::WIDGET_MIC_LAST;
pub const WIDGETS_NONE: Id = s::WIDGETS_NONE;
pub const WIDGETS_NO_MIC: Id = s::WIDGETS_NO_MIC;
/// A program of PimpMyShell's (`file`) isn't where the rest are.
pub fn part_missing(file: &str) -> String {
    let dir = pms::parts::path(file).and_then(|p| p.parent().map(pms::shown)).unwrap_or_default();
    s::PART_MISSING.with(&[("dir", &dir), ("so", &pms::parts::without(file))])
}
/// Under the list of what's in the taskbar.
pub const ORDER_NOTE: Id = s::ORDER_NOTE;
pub const COUNTERS: (Id, Id) = (s::COUNTERS, s::COUNTERS_ABOUT);
pub const LOOK: (Id, Id) = (s::LOOK, s::LOOK_ABOUT);
pub const TV: (Id, Id) = (s::TV, s::TV_ABOUT);
pub const TV_HELP: Id = s::TV_HELP;
pub const PIMPBOX: (Id, Id) = (s::PIMPBOX, s::PIMPBOX_ABOUT);
pub const NUMBER_ONE: (Id, Id) = (s::NUMBER_ONE, s::NUMBER_ONE_ABOUT);
pub const PIMPBOX_HELP: Id = s::PIMPBOX_HELP;
/// Where the order of what's in the taskbar is changed, for the pages of what's in it.
pub const ORDER_HINT: Id = s::ORDER_HINT;
pub const TWATCH: (Id, Id) = (s::TWATCH, s::TWATCH_ABOUT);
pub const TWATCH_FAVORITES: Id = s::TWATCH_FAVORITES;
/// And how the list is put in order, where it can be dragged.
pub const TWATCH_DRAG: Id = s::TWATCH_DRAG;
pub const TWATCH_AS_TEXT: Id = s::TWATCH_AS_TEXT;
pub const TWATCH_HIGHER: [(&str, Id); 2] = [("switch", s::TWATCH_HIGHER_SWITCH), ("stay", s::TWATCH_HIGHER_STAY)];
pub const TWATCH_CHECKS: [(&str, Id); 6] = [("60", s::TWATCH_CHECKS_60), ("120", s::TWATCH_CHECKS_120), ("300", s::TWATCH_CHECKS_300), ("600", s::TWATCH_CHECKS_600), ("900", s::TWATCH_CHECKS_900), ("1800", s::TWATCH_CHECKS_1800)];
pub const TWATCH_CHECK_EXTRAS: [(&str, Id); 5] = [("0", s::TWATCH_CHECK_EXTRAS_0), ("60", s::TWATCH_CHECK_EXTRAS_60), ("120", s::TWATCH_CHECK_EXTRAS_120), ("300", s::TWATCH_CHECK_EXTRAS_300), ("600", s::TWATCH_CHECK_EXTRAS_600)];
pub const TWATCH_QUALITIES: [(&str, Id); 5] = [("1080p60", s::TWATCH_QUALITIES_1080P60), ("720p60", s::TWATCH_QUALITIES_720P60), ("720p", s::TWATCH_QUALITIES_720P), ("480p", s::TWATCH_QUALITIES_480P), ("360p", s::TWATCH_QUALITIES_360P)];
pub const TWATCH_FRAME_RATES: [(&str, Id); 3] = [("0", s::TWATCH_FRAME_RATES_0), ("30", s::TWATCH_FRAME_RATES_30), ("15", s::TWATCH_FRAME_RATES_15)];
/// When the PIMPBOX is in efficiency mode.
pub const EFFICIENCY_PIMPBOX: [(&str, Id); 3] = [("closed", s::EFFICIENCY_PIMPBOX_CLOSED), ("always", s::EFFICIENCY_PIMPBOX_ALWAYS), ("never", s::EFFICIENCY_PIMPBOX_NEVER)];
/// The TV's channels, as the settings name them.
pub const TV_CHANNELS: [(&str, Id); 7] = [("1295", s::TV_CHANNELS_1295), ("2", s::TV_CHANNELS_2), ("3", s::TV_CHANNELS_3), ("4", s::TV_CHANNELS_4), ("5", s::TV_CHANNELS_5), ("37", s::TV_CHANNELS_37), ("304", s::TV_CHANNELS_304)];
/// How often PTV's bumps come (pms::iss::BUMPS).
pub const ISS_BUMPS: [(&str, Id); 3] = [("often", s::ISS_BUMPS_OFTEN), ("hourly", s::ISS_BUMPS_HOURLY), ("off", s::ISS_BUMPS_OFF)];
/// The heading over Number One's settings, on the TV page.
pub const TV_NUMBER_ONE: Id = s::TV_NUMBER_ONE;
pub const TWATCH_MEDIA_CONTROLS: [(&str, Id); 3] = [("never", s::TWATCH_MEDIA_CONTROLS_NEVER), ("unmuted", s::TWATCH_MEDIA_CONTROLS_UNMUTED), ("playing", s::TWATCH_MEDIA_CONTROLS_PLAYING)];
/// Above the Twatch TV page while there are favorites, video is wanted, and Streamlink isn't here.
pub const TWATCH_NO_STREAMLINK: (Id, Id) = (s::TWATCH_NO_STREAMLINK, s::TWATCH_NO_STREAMLINK_ABOUT);
pub const TWATCH_PICTURES: [(&str, Id); 4] = [("never", s::TWATCH_PICTURES_NEVER), ("initial", s::TWATCH_PICTURES_INITIAL), ("low_memory", s::TWATCH_PICTURES_LOW_MEMORY), ("always", s::TWATCH_PICTURES_ALWAYS)];
pub const RESET_ALL: (Id, Id) = (s::RESET_ALL, s::RESET_ALL_ABOUT);
pub const RESET_ALL_ASK: Id = s::RESET_ALL_ASK;
pub const TWATCH_NETWORK: Id = s::TWATCH_NETWORK;
pub const TWATCH_NOTICE: Id = s::TWATCH_NOTICE;
pub const NETWORK: (Id, Id) = (s::NETWORK, s::NETWORK_ABOUT);
pub const PROXY: (Id, Id) = (s::PROXY, s::PROXY_ABOUT);
pub const PROXY_ISOLATE: (Id, Id) = (s::PROXY_ISOLATE, s::PROXY_ISOLATE_ABOUT);
/// What a proxy here can't promise, and what can.
pub const PROXY_LIMITS: Id = s::PROXY_LIMITS;
/// The network settings' file won't load.
pub const NETWORK_UNREAD: Id = s::NETWORK_UNREAD;

/// A proxy is set, and nothing goes through it.
pub const PROXY_UNUSED: (Id, Id) = (s::PROXY_UNUSED, s::PROXY_UNUSED_ABOUT);
pub const PROXY_STRICT: Id = s::PROXY_STRICT;
pub const NETWORK_ELSE: Id = s::NETWORK_ELSE;
pub const HOTMIC: (Id, Id) = (s::HOTMIC, s::HOTMIC_ABOUT);
pub const TOASTS: (Id, Id) = (s::TOASTS, s::TOASTS_ABOUT);

#[cfg(test)]
mod tests {
    use super::*;

    /// UTF-8 read as Windows-1252 and saved again turns a middle dot, a bullet or a dash into pairs of odd letters. It still
    /// builds and runs, so nothing else would notice. (The pairs are spelled out by code point, or this file would trip.)
    #[test]
    fn no_garbled_text() {
        let garbled = [['\u{c2}', '\u{b7}'], ['\u{e2}', '\u{2014}'], ['\u{e2}', '\u{20ac}'], ['\u{c3}', '\u{a9}']].map(|p| p.iter().collect::<String>());
        let sources = [
            ("main.rs", include_str!("main.rs")),
            ("winui.rs", include_str!("winui.rs")),
            ("model.rs", include_str!("model.rs")),
            ("copy.rs", include_str!("copy.rs")),
            ("dollars.rs", include_str!("dollars.rs")),
            ("tooltip.rs", include_str!("../../pms-tap/src/tooltip.rs")),
            ("meters.rs", include_str!("../../pms-tap/src/meters.rs")),
            ("toast.rs", include_str!("../../pms-tap/src/toast.rs")),
            ("daemon meters.rs", include_str!("../../pms/src/meters.rs")),
            ("notices.rs", include_str!("../../pms/src/notices.rs")),
            ("tray.rs", include_str!("../../pms/src/tray.rs")),
        ];
        for (name, text) in sources {
            for g in &garbled {
                assert!(!text.contains(g.as_str()), "{name} has garbled text: {g}");
            }
        }
    }

    #[test]
    fn keys_and_style() {
        assert_eq!(key("interval_ms"), "interval_ms");
        assert_eq!(key("meter.names.disks"), "[[meter]] names");
        assert_eq!(key("start_glow.hover_ms"), "[start_glow] hover_ms");
        assert_eq!(key("iss.online"), "[iss] online");
        assert_eq!(key("look.tint"), "[backdrop] tint");
        assert_eq!(key("look.light.grain"), "[backdrop.light] grain");
        assert_eq!(key("counter.agg"), "[counters] agg");
        assert_eq!(setting("look.light.tint"), setting("look.tint"));
        // Titles are sentence case without a period; descriptions don't end in one either (they're fragments, or
        // end with a fragment).
        for id in ["theme", "update_check", "meter.show_when", "hotmic.broad_ms", "toasts.ignore", "look.tint"] {
            let (title, desc) = setting(id);
            assert!(!title.ends_with('.') && !desc.ends_with('.'), "{id}");
            assert!(title.chars().next().is_some_and(char::is_uppercase), "{id}");
        }
    }
}
