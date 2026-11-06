//! What the settings say: each setting's title and description, the same in both windows (egui in main.rs, WinUI 3 in
//! winui.rs). Written as Microsoft's style guide has it: sentence case, "you", short and active, no period after a
//! fragment.
//!
//! A setting is known by its id, `table.key` as the config file has it (`meter.unit`, `hotmic.color`), or `key` for
//! a top-level one; a few say different things for different meter styles (`meter.names.disks`).

/// The title and description of setting `id`. An id with nothing written here shows itself.
pub fn setting(id: &str) -> (&str, &str) {
    match id {
        // General
        "theme" => ("Theme", "Choose dark or light for the taskbar's look, or follow your Windows mode"),
        "meter_height" => ("Meter height", "How much of the taskbar's height meters use"),
        "interval_ms" => ("Update interval", "How often meters refresh, unless a meter has its own interval"),
        "software_gpus" => ("Include software graphics adapters", "Count Microsoft Basic Render Driver, which runs on the CPU, in GPU meters"),
        "max_fps" => ("Limit animation frame rate", "Cap ongoing animations, such as the microphone alert. When it's off, they run at your display's refresh rate"),
        "battery_fps" => ("Frame rate on battery", "A lower cap while your PC runs on battery. 0 means no cap"),
        "value_font" => ("Meter font", "A fixed-width font keeps digits from shifting. Leave it empty to use Cascadia Mono, or Consolas"),
        "update_check" => (
            "Check for updates automatically",
            "We check GitHub shortly after PimpMyShell starts, then every day or two. New versions download in the background, and you choose when to install them",
        ),
        "update_channel" => ("Update channel", "Stable gets a version once it has proven itself. Latest gets every release as soon as it's published"),

        // Meters
        "meter.label" => ("Name", "Shown next to the meter and in its tooltip"),
        "meter.group" => ("Group", "Meters in different groups are separated by a divider"),
        "meter.snug" => ("Attach to the previous meter", "No divider or space between this meter and the one before it"),
        "meter.caption" => ("Show name on the taskbar", "The name always appears in the tooltip"),
        "meter.style" => ("Style", "How the meter is drawn"),
        "meter.expr" => ("Expression", "A formula over counter names and built-in values, using + - * / ( ) min max abs. Pies and bars take one per slice or bar"),
        "meter.names" => ("Names", "One per slice or bar, shown in the tooltip"),
        "meter.names.disks" => ("Bar names", "What tooltips call each disk's two bars: write, then read"),
        "meter.names.nics" => ("Bar names", "What tooltips call each network adapter's two bars: upload, then download"),
        "meter.colors" => ("Colors", "One per slice or bar"),
        "meter.colors.cores" => ("Colors", "User time, then kernel time"),
        "meter.colors.disks" => ("Colors", "Write, then read"),
        "meter.colors.nics" => ("Colors", "Upload, then download"),
        "meter.color" => ("Color", "The meter's color"),
        "meter.max" => ("Full scale", "The value that fills the meter: a number or an expression. Choose None to scale automatically (a pie uses the sum of its slices)"),
        "meter.unit" => ("Unit", "%, bytes, bytes/s, or bits/s (sizes and rates scale to K, M, and G), or any other suffix"),
        "meter.decimals" => ("Decimal places", "Digits after the decimal point"),
        "meter.interval_ms" => ("Update interval", "How often this meter refreshes. By default, it uses the general update interval"),
        "meter.value" => ("Show value", "The value always appears in the tooltip"),
        "meter.tooltip" => ("Show tooltip", "Details appear when you point to the meter"),
        "meter.glow" => (
            "Glow",
            "Load glows with how busy each bar is now. Heat builds while a bar stays busy (full after about 8 seconds at 100%) and fades about 30 seconds after it stops",
        ),
        "meter.kernel" => ("Kernel time", "Where kernel time sits in each thread's bar: below user time, or above it"),
        "meter.bar_width" => ("Bar width", "Each bar's width before bars narrow to fit. When it's off, bars fit automatically"),
        "meter.vendors" => ("Label by manufacturer", "Label each GPU by its manufacturer, and color its bars to match"),
        "meter.vendor_label" => ("Manufacturer labels", "Short labels (NV, RAD) or stock tickers (NVDA, AMD)"),
        "meter.hold_seconds" => ("Hide idle GPUs after", "The GPU that was busy most recently stays visible. 0 keeps every GPU visible"),
        "meter.hold_seconds.disks" => ("Hide idle disks after", "The disk Windows is on stays visible. 0 keeps every disk visible"),
        "meter.hold_seconds.nics" => ("Hide idle adapters after", "The adapter your internet connection uses stays visible. 0 keeps every adapter visible"),
        "meter.engines_fall_off" => ("Hide idle engines", "Hide individual idle engines too, not just whole GPUs"),
        "meter.per_gpu" => (
            "One pie per GPU",
            "A pie for each graphics adapter with at least 1 GB of dedicated memory, in its manufacturer's color. We fill in the expression, full scale, and color",
        ),
        "meter.label_inside" => ("Name inside the pie", "Put the name inside the pie: never, only when space is short, or always"),
        "meter.pressure" => ("Pressure point", "How full the first slice gets before the glow around the pie grows stronger"),
        "meter.time_axis" => ("Time scale", "Logarithmic gives recent values the most room. Linear spaces every value evenly"),
        "meter.history_seconds" => ("History length", "How far back the graph goes"),
        "meter.show_above" => ("Show only above", "Hide the meter until its value reaches this number or expression"),
        "meter.show_when" => (
            "Show only when",
            "Hide the meter until another value reaches a threshold, and hide it again once that value drops back. For example, show the pagefile only when memory runs low",
        ),

        // Counters
        "counter.name" => ("Name", "What expressions call this counter"),
        "counter.path" => ("Path", "\\Object(instance)\\Counter, with * for every instance"),
        "counter.agg" => ("Combine", "How instances (*) combine. GPU engine matches Task Manager's GPU %"),
        "counter.default" => ("Fallback", "A value to use when the counter can't be read"),

        // Taskbar look
        "look.tint" => ("Tint", "Laid over the blurred desktop. Its transparency sets how much shows through"),
        "look.grain" => ("Grain", "Noise over the tint"),
        "look.gaps" => ("Fill between buttons", "Color the empty space between taskbar buttons"),
        "start_glow.color" => ("Color", "The glow's color"),
        "start_glow.hover" => ("Brightness on hover", "When you point to the Start button"),
        "start_glow.press" => ("Brightness when pressed", "While you press the Start button"),
        "start_glow.open" => ("Brightness while Start is open", "Also when you open Start with the Windows key"),
        "start_glow.hover_ms" => ("Fade time", "How long the glow takes to fade in and out"),
        "start_glow.press_ms" => ("Fade time when pressed", "Usually quicker"),
        "start_glow.hover_radius" => ("Size on hover", "How far the glow reaches, in pixels"),
        "start_glow.press_radius" => ("Size when pressed", "How far the glow reaches, in pixels"),
        "app_glow.hover" => ("Brightness on hover", "When you point to an app button"),
        "app_glow.press" => ("Brightness when pressed", "While you press an app button"),
        "app_glow.hover_ms" => ("Fade time", "How long the glow takes to fade in and out"),
        "app_glow.press_ms" => ("Fade time when pressed", "Usually quicker"),
        "app_glow.hover_radius" => ("Size on hover", "How far the glow reaches, in pixels"),
        "app_glow.press_radius" => ("Size when pressed", "How far the glow reaches, in pixels"),

        // Microphone alert
        "hotmic.color" => ("Alert color", "The color of the alert"),
        "hotmic.period_ms" => ("Pulse length", "One pulse of the alert. 0 keeps it steady"),
        "hotmic.low" => ("Lowest opacity", "How faint the pulse gets"),
        "hotmic.high" => ("Highest opacity", "How strong the pulse gets"),
        "hotmic.broad_ms" => ("Full taskbar alert", "How long the whole taskbar alerts before the alert settles next to the system tray"),
        "hotmic.ignore" => ("Excluded apps", "Apps that never trigger the alert. Enter part of an app's name or path"),

        // Notifications
        "toasts.duration_ms" => (
            "Display time",
            "How long a notification stays in front, or at most 4 seconds if more are waiting. Those that ask for longer get at least 25 seconds. Alarms, calls, and reminders stay until you respond",
        ),
        "toasts.style" => ("Card style", "Standard matches Windows. Pimped turns cards gold, with a sheen that sweeps across them"),
        "toasts.ignore" => ("Excluded apps", "Apps that keep their standard pop-ups. Enter part of an app's name or ID"),

        _ => (id, ""),
    }
}

/// The key the config file has for setting `id`: its last part (`meter.names.disks` is `names`).
pub fn key(id: &str) -> &str {
    let mut parts = id.split('.');
    let first = parts.next().unwrap_or(id);
    parts.next().unwrap_or(first)
}

/// What's in the logs, and what isn't (logfile.rs, and the daemon's lines on toasts).
pub const LOG_PRIVACY: &str = "Logs can include app and file names and error details. We leave out your Windows user name and what your notifications say, but check a log before you share it.";

/// The small print for what floats behind the settings window (dollars.rs).
pub const LEGAL: &str = "NOTICE REGARDING BACKGROUND IMAGERY. The words \"BUY $MSFT\" and all depictions of currency displayed in \
the background of this window (collectively, the \"Background\") are decorative, are provided solely for purposes of \
entertainment, and do not constitute, and shall not be construed as, financial, investment, legal, tax, or accounting advice, \
or as an offer, solicitation, or recommendation to purchase, sell, or hold any security or other financial instrument. The \
authors and contributors of PimpMyShell (the \"Authors\") are not registered investment advisers, broker-dealers, or \
fiduciaries, assume no duty to you in respect of the Background, and make no representation or warranty of any kind, express \
or implied, as to the merit, suitability, or future performance of any investment. Past performance is not indicative of \
future results. All investments involve risk, including the possible loss of principal. You are solely responsible for your \
own investment decisions and should consult a qualified, licensed professional before making them. To the fullest extent \
permitted by applicable law, the Authors disclaim all liability for any loss or damage arising from or in connection with \
any reliance on the Background. The Authors may or may not hold positions in the security referenced herein and are under no \
obligation to disclose or update the same. \"MSFT\" is the ticker symbol of Microsoft Corporation, which is not affiliated \
with, and has neither sponsored nor endorsed, PimpMyShell or the Background.";

/// Choices: each value as the file has it, and what the window calls it.
pub const THEMES: &[(&str, &str)] = &[("auto", "Use my Windows mode"), ("dark", "Dark"), ("light", "Light")];
pub const HEIGHTS: &[(&str, &str)] = &[("compact", "Compact (75%)"), ("tall", "Tall (88%)"), ("full", "Full")];
pub const CHANNELS: &[(&str, &str)] = &[("stable", "Stable"), ("latest", "Latest")];
pub const TOAST_STYLES: &[(&str, &str)] = &[("standard", "Standard"), ("pimped", "Pimped")];
pub const STYLES: &[(&str, &str)] = &[
    ("history", "History graph"),
    ("bar", "Bar"),
    ("pie", "Pie"),
    ("bars", "Bars"),
    ("cores", "CPU threads"),
    ("engines", "GPU engines"),
    ("disks", "Disks"),
    ("nics", "Network adapters"),
];
pub const UNITS: &[(&str, &str)] = &[("", "None"), ("%", "%"), ("bytes", "Bytes"), ("bytes/s", "Bytes/s"), ("bits/s", "Bits/s")];
pub const GLOWS: &[(&str, &str)] = &[("level", "Load"), ("heat", "Heat"), ("off", "Off")];
pub const KERNELS: &[(&str, &str)] = &[("bottom", "Below user time"), ("top", "Above user time")];
pub const VENDOR_LABELS: &[(&str, &str)] = &[("short", "Short (NV, RAD)"), ("ticker", "Tickers (NVDA, AMD)")];
pub const INSIDE: &[(&str, &str)] = &[("never", "Never"), ("auto", "When space is short"), ("always", "Always")];
pub const TIME_AXES: &[(&str, &str)] = &[("log", "Logarithmic"), ("linear", "Linear")];
pub const AGGS: &[(&str, &str)] = &[("sum", "Sum"), ("max", "Maximum"), ("min", "Minimum"), ("avg", "Average"), ("engine", "GPU engine")];

/// Pages: title, and what's on them.
pub const GENERAL: (&str, &str) = ("General", "");
pub const METERS: (&str, &str) = ("Meters", "Meters appear on the taskbar in this order, from left to right. When space runs short, the last ones hide first.");
pub const COUNTERS: (&str, &str) = (
    "Performance counters",
    "The Windows performance counters your meters read, by their English paths. Use their names in meter expressions, along with ram_total, vram_total, swap_total, swap_used, and cpu_count. To find paths, run typeperf -q.",
);
pub const LOOK: (&str, &str) = ("Taskbar look", "Replace the taskbar's standard acrylic with your own background, with a look for dark mode and one for light mode.");
pub const HOTMIC: (&str, &str) = (
    "Microphone alert",
    "Know when an app is using your microphone. The whole taskbar alerts first, then the alert settles next to the system tray and names the apps. An app that stops stays listed for 10 seconds.",
);
pub const TOASTS: (&str, &str) = ("Notifications", "Show notifications in the taskbar instead of as pop-ups. Their buttons still work, and they stay in notification center.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_style() {
        assert_eq!(key("interval_ms"), "interval_ms");
        assert_eq!(key("meter.names.disks"), "names");
        assert_eq!(key("start_glow.hover_ms"), "hover_ms");
        // Titles are sentence case without a period; descriptions don't end in one either (they're fragments, or
        // end with a fragment).
        for id in ["theme", "update_check", "meter.show_when", "hotmic.broad_ms", "toasts.ignore", "look.tint"] {
            let (title, desc) = setting(id);
            assert!(!title.ends_with('.') && !desc.ends_with('.'), "{id}");
            assert!(title.chars().next().is_some_and(char::is_uppercase), "{id}");
        }
    }
}
